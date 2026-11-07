import { api, clientId, errMsg } from "../shared/api";
import "./style.css";

const TOKEN_KEY = "exam_student_token";
const CID_KEY = "exam_student_client_id";
const PROFILE_KEY = "exam_student_profile";

function taskParam(): string {
  return new URLSearchParams(location.search).get("t") || "";
}

function tokenStoreKey(taskId: string): string {
  return `${TOKEN_KEY}:${taskId}`;
}

function readToken(): string | null {
  const t = taskParam();
  if (t) {
    return localStorage.getItem(tokenStoreKey(t));
  }
  return localStorage.getItem(TOKEN_KEY);
}

function writeToken(value: string, taskId?: string): void {
  const id = (taskId || taskParam()).trim();
  localStorage.removeItem(TOKEN_KEY);
  if (id) {
    localStorage.setItem(tokenStoreKey(id), value);
  } else {
    localStorage.setItem(TOKEN_KEY, value);
  }
}

function errCode(data: unknown): string {
  const e = data as { error?: { code?: string } };
  return e?.error?.code || "";
}

type Item = {
  question_id: string;
  qtype: string;
  stem: string;
  stem_html?: string;
  options?: Array<{ key: string; text: string }>;
  points?: number;
  answer?: unknown;
  response?: { kind: string; value: unknown };
  score?: unknown;
  correct?: unknown;
};

type Answer = { kind: string; value: unknown };

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function showValue(v: unknown): string {
  if (v == null || v === "") {
    return "未作答";
  }
  if (typeof v === "object" && v && "value" in (v as object)) {
    return showValue((v as Answer).value);
  }
  if (Array.isArray(v)) {
    return v.length ? v.map(String).join("、") : "未作答";
  }
  if (typeof v === "boolean") {
    return v ? "对" : "错";
  }
  return String(v);
}

const cid = clientId(CID_KEY);
let token = readToken();
let flash = "";
let saveNote = "答案将自动保存";
let paper: {
  title?: string;
  task_id?: string;
  status?: string;
  mode?: string;
  allow_early_submit?: boolean;
  closes_at?: string | null;
  task_status?: string;
  items?: Item[];
  answers?: Record<string, Answer>;
  released?: boolean;
  duration_minutes?: number;
  total_score?: number;
  class_name?: string;
  name?: string;
  student_no?: string;
} | null = null;
let result: Record<string, unknown> | null = null;
let brief: {
  task_id?: string;
  title?: string;
  mode?: string;
  status?: string;
  duration_minutes?: number;
  total_score?: number;
  classes?: string[];
  single_class?: boolean;
} | null = null;
let answers: Record<string, Answer> = {};
let saveTimer: number | null = null;
let clockTimer: number | null = null;
let debounceTimer: number | null = null;
let submitting = false;
let paperBound = false;

const root = document.getElementById("app")!;

function dropToken(): void {
  const ids = [taskParam(), String(paper?.task_id || ""), String(brief?.task_id || "")];
  for (const id of ids) {
    if (id) {
      localStorage.removeItem(tokenStoreKey(id));
    }
  }
  localStorage.removeItem(TOKEN_KEY);
  token = null;
}

function cacheKey(): string {
  return `exam_draft_${paper?.task_id || "cur"}_${cid}`;
}

function persistLocal(): void {
  try {
    localStorage.setItem(cacheKey(), JSON.stringify({ answers, at: Date.now() }));
  } catch {
    /* ignore quota */
  }
}

function restoreLocal(): Record<string, Answer> {
  try {
    const raw = localStorage.getItem(cacheKey());
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as { answers?: Record<string, Answer> };
    return parsed.answers || {};
  } catch {
    return {};
  }
}

function mergeAnswers(server: Record<string, Answer> | undefined): Record<string, Answer> {
  const local = restoreLocal();
  const out = { ...local, ...(server || {}) };
  for (const [k, v] of Object.entries(local)) {
    const cur = out[k];
    if (!cur || showValue(cur.value) === "未作答") {
      out[k] = v;
    }
  }
  return out;
}

async function scall<T = Record<string, unknown>>(path: string, opts: RequestInit = {}) {
  const headers = new Headers(opts.headers);
  const t = taskParam();
  if (t) {
    headers.set("x-exam-task", t);
  }
  const r = await api<T>(path, { ...opts, headers, token });
  if (r.status === 401 || errCode(r.data) === "TASK_MISMATCH") {
    dropToken();
  }
  return r;
}

function isAnswered(item: Item): boolean {
  const a = answers[item.question_id];
  if (!a) {
    return false;
  }
  return showValue(a.value) !== "未作答";
}

function collect(): Record<string, Answer> {
  const out: Record<string, Answer> = { ...answers };
  root.querySelectorAll<HTMLElement>("[data-qid]").forEach((box) => {
    const id = box.dataset.qid || "";
    const qtype = box.dataset.qtype || "single";
    if (qtype === "multiple") {
      const value = Array.from(box.querySelectorAll<HTMLInputElement>("input:checked")).map((i) => i.value);
      out[id] = { kind: "multiple", value };
    } else if (qtype === "single") {
      const picked = box.querySelector<HTMLInputElement>("input:checked");
      if (picked) {
        out[id] = { kind: "single", value: picked.value };
      }
    } else if (qtype === "judge") {
      const picked = box.querySelector<HTMLInputElement>("input:checked");
      if (picked) {
        out[id] = { kind: "judge", value: picked.value === "true" };
      }
    } else if (qtype === "fill") {
      const v = box.querySelector<HTMLInputElement>("input,textarea")?.value || "";
      out[id] = { kind: "fill", value: v };
    } else {
      const v = box.querySelector<HTMLTextAreaElement>("textarea")?.value || "";
      out[id] = { kind: "code", value: v };
    }
  });
  answers = out;
  persistLocal();
  return out;
}

function syncCard(): void {
  const items = paper?.items || [];
  const done = items.filter(isAnswered).length;
  const prog = document.getElementById("progress");
  if (prog) {
    prog.textContent = `已答 ${done}/${items.length}`;
  }
  items.forEach((it, i) => {
    const dot = document.querySelector<HTMLElement>(`.dot[data-jump="${i}"]`);
    if (dot) {
      dot.classList.toggle("on", isAnswered(it));
    }
  });
}

async function save(): Promise<void> {
  if (!token || paper?.status !== "in_progress") {
    return;
  }
  const r = await scall("/api/student/answers", {
    method: "PUT",
    body: JSON.stringify({ answers: collect() }),
  });
  saveNote = r.status < 400 ? `已保存 ${new Date().toLocaleTimeString()}` : "保存失败，已暂存在本机";
  const el = document.getElementById("save-note");
  if (el) {
    el.textContent = saveNote;
  }
}

function scheduleSave(): void {
  collect();
  syncCard();
  if (debounceTimer) {
    window.clearTimeout(debounceTimer);
  }
  debounceTimer = window.setTimeout(() => {
    void save();
  }, 600);
}

async function refreshPaper(): Promise<void> {
  const r = await scall<typeof paper>("/api/student/paper");
  if (r.status === 200) {
    paper = r.data;
    const wanted = taskParam();
    if (wanted && paper?.task_id && paper.task_id !== wanted) {
      dropToken();
      paper = null;
      result = null;
      return;
    }
    answers = mergeAnswers(paper?.answers);
    persistLocal();
    if (paper?.status === "submitted" || paper?.released) {
      const rr = await scall("/api/student/result");
      result = rr.status === 200 ? rr.data : null;
    }
  } else if (r.status === 401 || errCode(r.data) === "TASK_MISMATCH") {
    paper = null;
    result = null;
    flash = "";
  } else {
    flash = errMsg(r.data, "无法加载试卷");
    paper = null;
  }
}

function remainMs(): number {
  const t = paper?.closes_at;
  if (!t) {
    return Number.POSITIVE_INFINITY;
  }
  const ms = Date.parse(t) - Date.now();
  return Number.isNaN(ms) ? Number.POSITIVE_INFINITY : ms;
}

function remain(): string {
  const ms = remainMs();
  if (!Number.isFinite(ms)) {
    return "不计时";
  }
  if (ms <= 0) {
    return "已到时";
  }
  const m = Math.floor(ms / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  return `${m}:${String(s).padStart(2, "0")}`;
}

function optionChecked(item: Item, key: string): boolean {
  const a = answers[item.question_id];
  if (!a) {
    return false;
  }
  if (Array.isArray(a.value)) {
    return a.value.includes(key);
  }
  return a.value === key;
}

function choiceKeys(v: unknown): string[] {
  if (v == null || v === "") {
    return [];
  }
  if (typeof v === "object" && v && "value" in (v as object)) {
    return choiceKeys((v as Answer).value);
  }
  if (Array.isArray(v)) {
    return v.map(String);
  }
  if (typeof v === "boolean") {
    return [];
  }
  return [String(v)];
}

function reviewOptionsHtml(item: Item): string {
  if (item.qtype !== "single" && item.qtype !== "multiple") {
    return "";
  }
  const opts = item.options || [];
  if (!opts.length) {
    return "";
  }
  const mine = new Set(choiceKeys(item.response));
  const right = new Set(choiceKeys(item.answer));
  return opts
    .map((o) => {
      const picked = mine.has(o.key);
      const ok = right.has(o.key);
      const cls = ["opt", "ro", picked ? "mine" : "", ok ? "right" : ""].filter(Boolean).join(" ");
      const tag = ok ? "正确答案" : picked ? "你的选择" : "";
      return `<div class="${cls}"><span><b>${esc(o.key)}.</b> ${esc(o.text)}</span>${tag ? `<em>${esc(tag)}</em>` : ""}</div>`;
    })
    .join("");
}

function itemHtml(item: Item, idx: number, readonly: boolean): string {
  const opts = (item.options || [])
    .map((o) => {
      const type = item.qtype === "multiple" ? "checkbox" : "radio";
      const checked = optionChecked(item, o.key) ? "checked" : "";
      return `<label class="opt"><input type="${type}" name="${esc(item.question_id)}" value="${esc(o.key)}" ${checked} ${readonly ? "disabled" : ""} /><span><b>${esc(o.key)}.</b> ${esc(o.text)}</span></label>`;
    })
    .join("");
  let extra = "";
  if (item.qtype === "judge") {
    const v = answers[item.question_id]?.value;
    extra = `<label class="opt"><input type="radio" name="${esc(item.question_id)}" value="true" ${v === true ? "checked" : ""} ${readonly ? "disabled" : ""} /><span>对</span></label>
      <label class="opt"><input type="radio" name="${esc(item.question_id)}" value="false" ${v === false ? "checked" : ""} ${readonly ? "disabled" : ""} /><span>错</span></label>`;
  }
  if (item.qtype === "fill") {
    extra = `<input class="fill" ${readonly ? "readonly" : ""} value="${esc(answers[item.question_id]?.value || "")}" />`;
  }
  if (item.qtype === "code") {
    extra = `<textarea rows="8" ${readonly ? "readonly" : ""}>${esc(answers[item.question_id]?.value || "")}</textarea>`;
  }
  const pts = item.points != null ? `<span class="meta">（${esc(item.points)} 分）</span>` : "";
  const typeName =
    item.qtype === "multiple" ? "多选" : item.qtype === "judge" ? "判断" : item.qtype === "fill" ? "填空" : item.qtype === "code" ? "编程" : "单选";
  return `<article class="q" id="q-${idx}" data-qid="${esc(item.question_id)}" data-qtype="${esc(item.qtype)}">
    <h3>${idx + 1}. [${typeName}] ${esc(item.stem)} ${pts}</h3>
    ${opts}${extra}
  </article>`;
}

function resultHtml(): string {
  const items = (result?.items as Item[]) || [];
  const rows = items
    .map((it, i) => {
      const ok = it.correct === true || (Array.isArray(it.correct) && it.correct[0] === true);
      const mark = ok ? '<span class="ok">正确</span>' : '<span class="bad">待核 / 错误</span>';
      return `<article class="q">
        <h3>${i + 1}. ${esc(it.stem)} ${mark} ${esc(it.score ?? "")}/${esc(it.points ?? "")}</h3>
        ${reviewOptionsHtml(it)}
        <p>作答：${esc(showValue(it.response))}</p>
        <p>参考：${esc(showValue(it.answer))}</p>
      </article>`;
    })
    .join("");
  const retry =
    result?.mode === "practice"
      ? `<button class="ghost" id="retry" type="button">再练一套</button>`
      : "";
  return hallFrame(`<div class="sheet">
    <div class="stamp">已批改</div>
    <h1>${esc(result?.title || paper?.title || "成绩回发")}</h1>
    <p class="meta">总分 ${esc(result?.total_score)} · 客观 ${esc(result?.objective_score)}</p>
    ${rows}
    ${retry}
  </div>`);
}

function profile(): { class_name?: string; name?: string; student_no?: string } {
  try {
    return JSON.parse(localStorage.getItem(PROFILE_KEY) || "{}");
  } catch {
    return {};
  }
}

async function loadBrief(): Promise<void> {
  const t = taskParam();
  const r = await api<typeof brief>(`/api/student/brief${t ? `?t=${encodeURIComponent(t)}` : ""}`);
  if (r.status === 200) {
    brief = r.data;
  } else {
    brief = null;
    if (!token) {
      flash = errMsg(r.data, "没有可进入的考试");
    }
  }
}

async function lookupIdentity(form: HTMLFormElement): Promise<void> {
  const fd = new FormData(form);
  const class_name = String(fd.get("class_name") || "").trim();
  const name = String(fd.get("name") || "").trim();
  const student_no = String(fd.get("student_no") || "").trim();
  if (!name && !student_no) {
    return;
  }
  const t = taskParam() || brief?.task_id || "";
  const q = new URLSearchParams();
  if (t) q.set("t", t);
  if (class_name) q.set("class_name", class_name);
  if (name) q.set("name", name);
  if (student_no) q.set("student_no", student_no);
  const r = await api<{ class_name?: string; name?: string; student_no?: string }>(
    `/api/student/lookup?${q.toString()}`,
  );
  if (r.status >= 400) {
    flash = errMsg(r.data, "未匹配到学生");
    const box = document.querySelector(".err");
    if (box) box.textContent = flash;
    return;
  }
  const cls = form.querySelector<HTMLInputElement>("input[name=class_name]");
  const nm = form.querySelector<HTMLInputElement>("input[name=name]");
  const no = form.querySelector<HTMLInputElement>("input[name=student_no]");
  if (cls && r.data.class_name) cls.value = r.data.class_name;
  if (nm && r.data.name) nm.value = r.data.name;
  if (no && r.data.student_no) no.value = r.data.student_no;
  flash = "";
  const box = document.querySelector(".err");
  if (box) box.textContent = "";
}

function bindPaper(): void {
  if (!paperBound) {
    root.addEventListener("change", onPaperEdit);
    root.addEventListener("input", onPaperEdit);
    paperBound = true;
  }
  root.querySelectorAll("[data-jump]").forEach((el) => {
    el.addEventListener("click", () => {
      const i = (el as HTMLElement).dataset.jump;
      document.getElementById(`q-${i}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  });
  root.querySelector("#submit")?.addEventListener("click", () => void trySubmit());
}

function onPaperEdit(e: Event): void {
  const t = e.target as HTMLElement | null;
  if (!t || !t.closest("[data-qid]")) {
    return;
  }
  scheduleSave();
}

function unanswered(): Item[] {
  return (paper?.items || []).filter((it) => !isAnswered(it));
}

async function trySubmit(): Promise<void> {
  collect();
  const miss = unanswered();
  const msg = miss.length
    ? `还有 ${miss.length} 题未作答，确认交卷？`
    : "确认交卷？提交后不能再改。";
  if (!window.confirm(msg)) {
    return;
  }
  await autoSubmit();
}

function hallFrame(inner: string, title = "教室考试"): string {
  return `<div class="hall">
    <header class="bar">
      <div>
        <strong>${esc(title)}</strong>
        <div class="save">局域网考场</div>
      </div>
    </header>
    ${inner}
  </div>`;
}

function render(): void {
  if (saveTimer) {
    window.clearInterval(saveTimer);
    saveTimer = null;
  }
  if (clockTimer) {
    window.clearInterval(clockTimer);
    clockTimer = null;
  }
  if (!token) {
    const open = brief?.status === "published" || brief?.status === "released";
    if (brief && !open) {
      const pending = brief.status === "draft";
      root.innerHTML = hallFrame(`<div class="sheet">
        <div class="stamp">${pending ? "未开放" : "已结束"}</div>
        <h1>${esc(brief.title || "考场试卷")}</h1>
        <p class="meta">${pending ? "这场考试尚未开放。请确认答卷入口是否对应已发布的任务。" : "这场考试已经结束。成绩在教师公开后再从本场入口查看。"}</p>
        ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
      </div>`);
      return;
    }
    const p = profile();
    const classes = brief?.classes || [];
    const classVal = p.class_name || (brief?.single_class ? String(classes[0] || "") : "");
    const classField = brief?.single_class
      ? `<label class="field">班级<input name="class_name" value="${esc(classVal)}" readonly /></label>`
      : `<label class="field">班级<input name="class_name" list="class-list" value="${esc(classVal)}" /></label>
         <datalist id="class-list">${classes.map((c) => `<option value="${esc(c)}"></option>`).join("")}</datalist>`;
    root.innerHTML = hallFrame(`<div class="enter">
      <div class="paper-head">
        <div class="school">教室考试</div>
        <h1>${esc(brief?.title || "考场试卷")}</h1>
        <div class="rule">
          <span>总分 ${esc(brief?.total_score ?? "—")} 分</span>
          <span>时间 ${esc(brief?.duration_minutes ?? "—")} 分钟</span>
        </div>
      </div>
      ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
      <p class="meta">先填写身份。姓名或学号填一项即可带出另一项；三项齐全后才能领取试卷。</p>
      <form id="enter">
        <div class="id-grid">
          ${classField}
          <label class="field">姓名<input name="name" value="${esc(p.name || "")}" /></label>
          <label class="field">学号<input name="student_no" value="${esc(p.student_no || "")}" /></label>
        </div>
        <p><button type="submit">领取试卷</button></p>
      </form>
    </div>`);
    root.querySelector("#enter")?.addEventListener("submit", (e) => {
      e.preventDefault();
      void onEnter(e.target as HTMLFormElement);
    });
    const form = root.querySelector<HTMLFormElement>("#enter");
    form?.querySelectorAll("input[name=name], input[name=student_no]").forEach((el) => {
      el.addEventListener("blur", () => {
        if (form) void lookupIdentity(form);
      });
    });
    return;
  }
  if (result && (paper?.released || paper?.mode === "practice") && (result as { items?: unknown }).items) {
    root.innerHTML = resultHtml();
    root.querySelector("#retry")?.addEventListener("click", () => {
      dropToken();
      paper = null;
      result = null;
      render();
    });
    return;
  }
  if (paper?.status === "submitted") {
    root.innerHTML = hallFrame(`<div class="sheet">
      <div class="stamp">已提交</div>
      <h1>答卷已交</h1>
      <p class="meta">考试成绩在教师公开后再显示。练习模式可重新进入再练。</p>
      ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
      <button class="ghost" id="leave" type="button">返回入口</button>
    </div>`);
    root.querySelector("#leave")?.addEventListener("click", () => {
      dropToken();
      paper = null;
      render();
    });
    return;
  }
  const items = paper?.items || [];
  const canSubmit = paper?.allow_early_submit;
  const done = items.filter(isAnswered).length;
  const dots = items
    .map(
      (it, i) =>
        `<button type="button" class="dot ${isAnswered(it) ? "on" : ""}" data-jump="${i}">${i + 1}</button>`,
    )
    .join("");
  root.innerHTML = `<div class="exam">
    <header class="bar">
      <div>
        <strong>${esc(paper?.title || (paper?.mode === "practice" ? "练习卷" : "考试卷"))}</strong>
        <div class="save" id="save-note">${esc(saveNote)}</div>
      </div>
      <div>
        <div class="clock" id="clock">${esc(remain())}</div>
        <div class="save" id="progress">已答 ${done}/${items.length}</div>
      </div>
      ${canSubmit ? `<button id="submit" type="button">交卷</button>` : `<span class="save">未开放提前交卷</span>`}
    </header>
    <div class="workspace">
      <div class="paper">
        ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
        <div class="paper-head">
          <div class="school">教室考试</div>
          <h1>${esc(paper?.title || (paper?.mode === "practice" ? "练习卷" : "考试卷"))}</h1>
          <div class="rule">
            <span>总分 ${esc(paper?.total_score ?? "—")} 分</span>
            <span>时间 ${esc(paper?.duration_minutes ?? "—")} 分钟</span>
          </div>
        </div>
        <div class="id-locked">
          <div>班级 <b>${esc(paper?.class_name || profile().class_name || "—")}</b></div>
          <div>姓名 <b>${esc(paper?.name || profile().name || "—")}</b></div>
          <div>学号 <b>${esc(paper?.student_no || profile().student_no || "—")}</b></div>
        </div>
        ${items.map((it, i) => itemHtml(it, i, false)).join("")}
      </div>
      <aside class="sheet-card">
        <p class="meta">答题卡 · 点击跳转</p>
        <div class="sheet-grid">${dots}</div>
      </aside>
    </div>
  </div>`;
  bindPaper();
  saveTimer = window.setInterval(() => {
    void save();
  }, 15000);
  clockTimer = window.setInterval(() => {
    const el = document.getElementById("clock");
    if (el) {
      el.textContent = remain();
    }
    if (remain() === "已到时") {
      void autoSubmit();
    }
  }, 1000);
}

async function autoSubmit(): Promise<void> {
  if (submitting) {
    return;
  }
  submitting = true;
  await save();
  const r = await scall("/api/student/submit", { method: "POST", body: "{}" });
  if (r.status >= 400) {
    flash = errMsg(r.data, "交卷失败");
    submitting = false;
    render();
    return;
  }
  paper = { ...(paper || {}), status: "submitted" };
  result = r.data;
  flash = "";
  try {
    localStorage.removeItem(cacheKey());
  } catch {
    /* ignore */
  }
  render();
}

async function onEnter(form: HTMLFormElement): Promise<void> {
  const fd = new FormData(form);
  const payload = {
    class_name: String(fd.get("class_name") || "").trim(),
    name: String(fd.get("name") || "").trim() || undefined,
    student_no: String(fd.get("student_no") || "").trim() || undefined,
    client_id: cid,
    task_id: taskParam() || brief?.task_id || undefined,
  };
  if (!payload.name || !payload.student_no) {
    flash = "请先填姓名或学号，系统带出另一项后再领取试卷";
    render();
    return;
  }
  localStorage.setItem(
    PROFILE_KEY,
    JSON.stringify({ class_name: payload.class_name, name: payload.name, student_no: payload.student_no }),
  );
  const r = await api<Record<string, unknown>>("/api/student/enter", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  if (r.status >= 400) {
    flash = errMsg(r.data, "未能进入");
    render();
    return;
  }
  token = String(r.data.token);
  writeToken(token, String(r.data.task_id || payload.task_id || ""));
  paper = r.data;
  localStorage.setItem(
    PROFILE_KEY,
    JSON.stringify({
      class_name: r.data.class_name || payload.class_name,
      name: r.data.name || payload.name,
      student_no: r.data.student_no || payload.student_no,
    }),
  );
  answers = mergeAnswers((r.data.answers as Record<string, Answer>) || {});
  persistLocal();
  flash = "";
  if (paper?.status === "submitted" || paper?.released) {
    const rr = await scall("/api/student/result");
    result = rr.status === 200 ? rr.data : r.data;
  }
  render();
}

async function boot(): Promise<void> {
  await loadBrief();
  token = readToken();
  if (token) {
    await refreshPaper();
    const wanted = taskParam() || brief?.task_id || "";
    if (wanted && paper?.task_id && paper.task_id !== wanted) {
      dropToken();
      paper = null;
      result = null;
    }
  }
  render();
}

void boot();
