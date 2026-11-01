import { api, clientId, errMsg } from "../shared/api";

const TOKEN_KEY = "exam_student_token";
const CID_KEY = "exam_student_client_id";

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

const cid = clientId(CID_KEY);
let token = localStorage.getItem(TOKEN_KEY);
let flash = "";
let paper: {
  status?: string;
  mode?: string;
  allow_early_submit?: boolean;
  closes_at?: string | null;
  task_status?: string;
  items?: Item[];
  answers?: Record<string, Answer>;
  released?: boolean;
} | null = null;
let result: Record<string, unknown> | null = null;
let answers: Record<string, Answer> = {};
let saveTimer: number | null = null;
let clockTimer: number | null = null;

const root = document.getElementById("app")!;

async function scall<T = Record<string, unknown>>(path: string, opts: RequestInit = {}) {
  const r = await api<T>(path, { ...opts, token });
  if (r.status === 401) {
    token = null;
    localStorage.removeItem(TOKEN_KEY);
  }
  return r;
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
  return out;
}

async function save(): Promise<void> {
  if (!token || paper?.status !== "in_progress") {
    return;
  }
  await scall("/api/student/answers", {
    method: "PUT",
    body: JSON.stringify({ answers: collect() }),
  });
}

async function refreshPaper(): Promise<void> {
  const r = await scall<typeof paper>("/api/student/paper");
  if (r.status === 200) {
    paper = r.data;
    answers = { ...(paper?.answers || {}) };
    if (paper?.status === "submitted" || paper?.released) {
      const rr = await scall("/api/student/result");
      result = rr.status === 200 ? rr.data : null;
    }
  } else {
    flash = errMsg(r.data, "无法加载试卷");
    paper = null;
  }
}

function remain(): string {
  const t = paper?.closes_at;
  if (!t) {
    return "";
  }
  const ms = Date.parse(t) - Date.now();
  if (Number.isNaN(ms)) {
    return "";
  }
  if (ms <= 0) {
    return "已到时";
  }
  const m = Math.floor(ms / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  return `剩余 ${m}:${String(s).padStart(2, "0")}`;
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

function itemHtml(item: Item, idx: number, readonly: boolean): string {
  const opts = (item.options || [])
    .map((o) => {
      const type = item.qtype === "multiple" ? "checkbox" : "radio";
      const checked = optionChecked(item, o.key) ? "checked" : "";
      return `<label class="opt"><input type="${type}" name="${esc(item.question_id)}" value="${esc(o.key)}" ${checked} ${readonly ? "disabled" : ""} /> <span>${esc(o.key)}. ${esc(o.text)}</span></label>`;
    })
    .join("");
  let extra = "";
  if (item.qtype === "judge") {
    const v = answers[item.question_id]?.value;
    extra = `<label class="opt"><input type="radio" name="${esc(item.question_id)}" value="true" ${v === true ? "checked" : ""} ${readonly ? "disabled" : ""} /> 对</label>
      <label class="opt"><input type="radio" name="${esc(item.question_id)}" value="false" ${v === false ? "checked" : ""} ${readonly ? "disabled" : ""} /> 错</label>`;
  }
  if (item.qtype === "fill") {
    extra = `<input ${readonly ? "readonly" : ""} value="${esc(answers[item.question_id]?.value || "")}" />`;
  }
  if (item.qtype === "code") {
    extra = `<textarea rows="8" ${readonly ? "readonly" : ""}>${esc(answers[item.question_id]?.value || "")}</textarea>`;
  }
  const pts = item.points != null ? `<span class="meta">（${esc(item.points)} 分）</span>` : "";
  return `<article class="q" data-qid="${esc(item.question_id)}" data-qtype="${esc(item.qtype)}">
    <h3>${idx + 1}. ${esc(item.stem)} ${pts}</h3>
    ${opts}${extra}
  </article>`;
}

function resultHtml(): string {
  const items = (result?.items as Item[]) || [];
  const rows = items
    .map((it, i) => {
      const ok = it.correct === true || (Array.isArray(it.correct) && it.correct[0] === true);
      const mark = ok ? '<span class="ok">正确</span>' : '<span class="bad">待核/错误</span>';
      return `<article class="q">
        <h3>${i + 1}. ${esc(it.stem)} ${mark} ${esc(it.score ?? "")}/${esc(it.points ?? "")}</h3>
        <p>作答：${esc(JSON.stringify(it.response))}</p>
        <p>参考：${esc(JSON.stringify(it.answer))}</p>
      </article>`;
    })
    .join("");
  const retry =
    result?.mode === "practice"
      ? `<button class="ghost" id="retry">再练一套</button>`
      : "";
  return `<div class="sheet">
    <div class="stamp">已批改</div>
    <h1>成绩回发</h1>
    <p class="meta">总分 ${esc(result?.total_score)} · 客观 ${esc(result?.objective_score)} · ${esc(result?.delivery)}</p>
    ${rows}
    ${retry}
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
    root.innerHTML = `<div class="sheet">
      <h1>进入考场</h1>
      <p class="meta">班级必填，姓名或学号至少一项。本机标识 ${esc(cid)}</p>
      ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
      <form id="enter">
        <label>班级<input name="class_name" required /></label>
        <label>姓名<input name="name" /></label>
        <label>学号<input name="student_no" /></label>
        <label>任务编号（可空，默认当前场次）<input name="task_id" /></label>
        <button type="submit">领取试卷</button>
      </form>
    </div>`;
    root.querySelector("#enter")?.addEventListener("submit", (e) => {
      e.preventDefault();
      void onEnter(e.target as HTMLFormElement);
    });
    return;
  }
  if (result && (paper?.released || paper?.mode === "practice") && (result as { items?: unknown }).items) {
    root.innerHTML = resultHtml();
    root.querySelector("#retry")?.addEventListener("click", () => {
      localStorage.removeItem(TOKEN_KEY);
      token = null;
      paper = null;
      result = null;
      render();
    });
    return;
  }
  if (paper?.status === "submitted") {
    root.innerHTML = `<div class="sheet">
      <div class="stamp">已提交</div>
      <h1>答卷已交</h1>
      <p class="meta">考试成绩在教师公开后再显示。练习模式可重新进入再练。</p>
      ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
      <button class="ghost" id="leave">返回入口</button>
    </div>`;
    root.querySelector("#leave")?.addEventListener("click", () => {
      localStorage.removeItem(TOKEN_KEY);
      token = null;
      paper = null;
      render();
    });
    return;
  }
  const items = paper?.items || [];
  const canSubmit = paper?.allow_early_submit;
  root.innerHTML = `<div class="sheet">
    <h1>试卷</h1>
    <p class="meta" id="clock">${esc(paper?.mode)} · ${remain()} · 自动保存每 15 秒</p>
    ${flash ? `<p class="err">${esc(flash)}</p>` : ""}
    ${items.map((it, i) => itemHtml(it, i, false)).join("")}
    ${canSubmit ? `<button id="submit">交卷</button>` : `<p class="meta">教师未开放提前交卷，到时将自动收卷。</p>`}
  </div>`;
  saveTimer = window.setInterval(() => {
    void save();
  }, 15000);
  clockTimer = window.setInterval(() => {
    const el = document.getElementById("clock");
    if (el) {
      el.textContent = `${paper?.mode || ""} · ${remain()} · 自动保存每 15 秒`;
    }
    if (remain() === "已到时") {
      void autoSubmit();
    }
  }, 1000);
  root.querySelector("#submit")?.addEventListener("click", () => void autoSubmit());
}

async function autoSubmit(): Promise<void> {
  await save();
  const r = await scall("/api/student/submit", { method: "POST", body: "{}" });
  if (r.status >= 400) {
    flash = errMsg(r.data, "交卷失败");
    render();
    return;
  }
  paper = { ...(paper || {}), status: "submitted" };
  result = r.data;
  flash = "";
  render();
}

async function onEnter(form: HTMLFormElement): Promise<void> {
  const fd = new FormData(form);
  const payload = {
    class_name: String(fd.get("class_name") || "").trim(),
    name: String(fd.get("name") || "").trim() || undefined,
    student_no: String(fd.get("student_no") || "").trim() || undefined,
    client_id: cid,
    task_id: String(fd.get("task_id") || "").trim() || undefined,
  };
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
  localStorage.setItem(TOKEN_KEY, token);
  paper = r.data;
  answers = { ...((r.data.answers as Record<string, Answer>) || {}) };
  flash = "";
  if (paper?.status === "submitted" || paper?.released) {
    const rr = await scall("/api/student/result");
    result = rr.status === 200 ? rr.data : r.data;
  }
  render();
}

if (token) {
  void refreshPaper().then(render);
} else {
  render();
}
