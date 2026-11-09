import { api, errMsg } from "../shared/api";
import "./style.css";

type Tab = "banks" | "roster" | "papers" | "tasks" | "marking" | "grades";

const TOKEN_KEY = "exam_teacher_token";
const QTYPES: [string, string][] = [
  ["single", "单选"],
  ["multiple", "多选"],
  ["judge", "判断"],
  ["fill", "填空"],
  ["code", "编程"],
];

const AUTO_GRADE_WARN =
  "打开后将按参考答案自动批改填空题。参考答案为空的填空仍需手工给分。编程题始终手工。学生答案与参考不完全一致可能错判。确定打开？";

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function clipTitle(text: unknown): string {
  const full = String(text ?? "");
  return `<span class="clip-title" title="${esc(full)}">${esc(full)}</span>`;
}

function qlabel(t: string): string {
  return QTYPES.find(([k]) => k === t)?.[1] || t;
}

function modeLabel(m: unknown): string {
  return m === "practice" ? "练习" : "考试";
}

function statusLabel(s: unknown): string {
  const map: Record<string, string> = {
    draft: "草稿",
    published: "进行中",
    closed: "已结束",
    graded: "已批改",
    released: "已公开",
    not_entered: "未进场",
    in_progress: "作答中",
    submitted: "已交卷",
  };
  return map[String(s)] || String(s ?? "");
}

function badgeFor(s: unknown): string {
  const v = String(s);
  if (v === "published" || v === "in_progress" || v === "released" || v === "ok") return "ok";
  if (v === "draft" || v === "not_entered" || v === "graded") return "warn";
  if (v === "closed" || v === "submitted") return "bad";
  return "";
}

function isHistory(status: unknown): boolean {
  const v = String(status);
  return v === "closed" || v === "graded" || v === "released";
}

function uniqueClasses(list: Array<Record<string, unknown>>): string[] {
  return Array.from(new Set(list.map((s) => String(s.class_name || "")).filter(Boolean)));
}

function childClassName(): string {
  const rosterPeople = ((roster?.students as Array<Record<string, unknown>>) || []);
  const classes = uniqueClasses(rosterPeople);
  if (classes.length === 1) {
    return classes[0];
  }
  const title = String(task?.title || "");
  const mark = " · ";
  const i = title.lastIndexOf(mark);
  if (i >= 0) {
    return title.slice(i + mark.length);
  }
  const named = uniqueClasses(students)
    .filter((c) => c && (title === c || title.endsWith(c)))
    .sort((a, b) => b.length - a.length);
  return named[0] || "";
}

function pageSizeFor(kind: "stu" | "q" | "mon"): number {
  const h = window.innerHeight || 800;
  if (kind === "stu" || kind === "mon") {
    const fitted = kind === "mon" ? pageFit.mon : pageFit.stu;
    if (fitted > 0) return fitted;
    return Math.max(8, Math.min(28, Math.floor((h - 400) / 36)));
  }
  const fitted = tab === "papers" ? pageFit.paperq : pageFit.bankq;
  if (fitted > 0) return fitted;
  return Math.max(3, Math.min(12, Math.floor((h - 300) / 120)));
}

function pagerHtml(which: string, page: number, pages: number, total: number): string {
  if (total === 0) {
    return "";
  }
  return `<div class="pager">
    <button type="button" data-page="${esc(which)}:prev" ${page <= 1 ? "disabled" : ""}>上一页</button>
    <span>第 ${esc(page)} / ${esc(pages)} 页 · 共 ${esc(total)}</span>
    <button type="button" data-page="${esc(which)}:next" ${page >= pages ? "disabled" : ""}>下一页</button>
  </div>`;
}

function boxHeight(el: Element): number {
  const st = window.getComputedStyle(el);
  return (
    (el as HTMLElement).offsetHeight +
    (Number.parseFloat(st.marginTop) || 0) +
    (Number.parseFloat(st.marginBottom) || 0)
  );
}

function packQuestionStarts(box: HTMLElement): number[] {
  const avail = box.clientHeight;
  const sheet = box.querySelector(".qsheet");
  if (!sheet || avail < 24) {
    return [0];
  }
  const items: { height: number; hadHeader: boolean }[] = [];
  let pending = 0;
  let pendingHeader = false;
  for (const kid of Array.from(sheet.children)) {
    const h = boxHeight(kid);
    if (kid.classList.contains("qitem")) {
      items.push({ height: pending + h, hadHeader: pendingHeader });
      pending = 0;
      pendingHeader = false;
    } else {
      pending += h;
      pendingHeader = true;
    }
  }
  if (!items.length) {
    return [0];
  }
  const h3 = sheet.querySelector("h3");
  const headerH = h3 ? boxHeight(h3) : 28;
  const starts = [0];
  let acc = 0;
  for (let i = 0; i < items.length; i++) {
    const h = items[i].height;
    if (acc > 0 && acc + h > avail + 4) {
      starts.push(i);
      acc = (items[i].hadHeader ? 0 : headerH) + items[i].height;
    } else {
      acc += h;
    }
  }
  return starts;
}

function questionSlice(
  packing: boolean,
  starts: number[],
  page: number,
  total: number,
): { from: number; count: number; pages: number } {
  if (!total) {
    return { from: 0, count: 0, pages: 1 };
  }
  if (packing) {
    return { from: 0, count: total, pages: Math.max(1, starts.length) };
  }
  const pages = Math.max(1, starts.length);
  const p = Math.min(Math.max(1, page), pages);
  const from = starts[p - 1] ?? 0;
  const end = starts[p] ?? total;
  return { from, count: Math.max(0, end - from), pages };
}

function scoreEditing(): boolean {
  return document.activeElement instanceof HTMLInputElement && document.activeElement.classList.contains("score-inline");
}

function fitVisibleLists(): void {
  if (fittingPages) return;
  if (scoreEditing()) return;
  if (tab !== "roster" && tab !== "banks" && tab !== "papers" && tab !== "tasks") return;
  fittingPages = true;
  pageFitCap = {};
  try {
    for (let i = 0; i < 3; i++) {
      if (!fitOnce()) break;
    }
  } finally {
    fittingPages = false;
  }
}

function fitOnce(): boolean {
  const key: "stu" | "bankq" | "paperq" | "mon" =
    tab === "roster" ? "stu" : tab === "papers" ? "paperq" : tab === "tasks" ? "mon" : "bankq";
  const box = root.querySelector(`[data-fit="${key}"]`) as HTMLElement | null;
  if (!box) return false;
  const avail = box.clientHeight;
  if (avail < 24) return false;
  const overflowing = box.scrollHeight > avail + 4;
  let n = 0;
  if (key === "stu" || key === "mon") {
    const rows = Array.from(box.querySelectorAll("tbody tr"));
    if (!rows.length || rows[0].querySelector(".muted")) return false;
    const head = box.querySelector("thead");
    const headH = head ? boxHeight(head) : 28;
    if (overflowing) {
      let acc = headH;
      for (const row of rows) {
        const h = boxHeight(row);
        if (n > 0 && acc + h > avail) break;
        acc += h;
        n += 1;
      }
      pageFitCap[key] = n;
    } else {
      const avg = rows.reduce((s, r) => s + boxHeight(r), 0) / rows.length;
      n = Math.floor((avail - headH) / Math.max(avg, 28));
      const cap = pageFitCap[key];
      if (cap && n > cap) n = cap;
    }
    n = Math.max(key === "mon" ? 1 : 4, Math.min(60, n));
  } else {
    const packing = key === "paperq" ? paperNeedPack : bankNeedPack;
    if (!packing) {
      return false;
    }
    if (!box.querySelector(".qitem")) {
      return false;
    }
    const starts = packQuestionStarts(box);
    if (key === "paperq") {
      paperPackStarts = starts;
      paperNeedPack = false;
      if (paperQPage > starts.length) {
        paperQPage = starts.length;
      }
    } else {
      bankPackStarts = starts;
      bankNeedPack = false;
      if (bankQPage > starts.length) {
        bankQPage = starts.length;
      }
    }
    render();
    return true;
  }
  if (n === pageFit[key]) return false;
  pageFit[key] = n;
  render();
  return true;
}

function chineseIndex(n: number): string {
  return ["一", "二", "三", "四", "五", "六", "七", "八"][n - 1] || String(n);
}

function typeSection(t: string): string {
  if (t === "single" || t === "multiple") return "选择题";
  if (t === "judge") return "判断题";
  if (t === "fill") return "填空题";
  if (t === "code") return "编程题";
  return qlabel(t);
}

function formatAnswerHtml(q: Record<string, unknown>): string {
  const a = q.answer as { kind?: string; value?: unknown } | undefined;
  if (!a) return "";
  if (a.kind === "single") return `答案：${esc(a.value)}`;
  if (a.kind === "multiple") {
    const v = Array.isArray(a.value) ? (a.value as string[]).join("") : String(a.value ?? "");
    return `答案：${esc(v)}`;
  }
  if (a.kind === "judge") {
    return a.value === true
      ? `答案：<span class="check-mark">&#10003;</span>`
      : "答案：×";
  }
  if (a.kind === "fill") {
    const v = Array.isArray(a.value) ? (a.value as string[]).join("|") : String(a.value ?? "");
    return `答案：${esc(v)}`;
  }
  if (a.kind === "code") return `参考答案：<br><pre>${esc(a.value)}</pre>`;
  return esc(JSON.stringify(a));
}

function stripInlineChoiceHtml(html: string): string {
  const m = html.match(/<br\s*\/?>\s*A\s*[.．]/i);
  if (!m || m.index == null) {
    return html;
  }
  return html.slice(0, m.index).replace(/(?:<br\s*\/?>\s*)+$/i, "").trimEnd();
}

function displayStemHtml(
  stemHtml: unknown,
  stem: unknown,
  hasOptions: boolean,
  fallbackPrefix = "",
): string {
  let html = String(stemHtml || "");
  if (hasOptions) {
    html = stripInlineChoiceHtml(html);
  }
  if (html.includes("<")) {
    return html;
  }
  return `<p>${fallbackPrefix}${esc(stem)}</p>`;
}

function answerToInput(q: Record<string, unknown>): string {
  const a = q.answer as { kind?: string; value?: unknown } | undefined;
  if (!a) return "";
  if (a.kind === "multiple" && Array.isArray(a.value)) return (a.value as string[]).join(",");
  if (a.kind === "judge") return a.value === true ? "true" : "false";
  if (a.kind === "fill" && Array.isArray(a.value)) return (a.value as string[]).join("|");
  return String(a.value ?? "");
}

let token = localStorage.getItem(TOKEN_KEY);
let tab: Tab = "banks";
let lan: { addresses?: string[]; port?: number } | null = null;
let banks: Array<Record<string, unknown>> = [];
let bankId = "";
let bank: Record<string, unknown> | null = null;
let students: Array<Record<string, unknown>> = [];
let classes: Array<Record<string, unknown>> = [];
let classFilter = "";
const selectedStu = new Set<string>();
let papers: Array<Record<string, unknown>> = [];
let paperId = "";
let paperQs: Array<Record<string, unknown>> = [];
let paperPreviewErr = "";
let showPaperForm = false;
let rosterPage = 1;
let bankQPage = 1;
let paperQPage = 1;
let paperPackStarts: number[] = [0];
let paperNeedPack = true;
let bankPackStarts: number[] = [0];
let bankNeedPack = true;
let showTaskForm = false;
let monPage = 1;
const pageFit: Record<"stu" | "bankq" | "paperq" | "mon", number> = { stu: 0, bankq: 0, paperq: 0, mon: 0 };
let pageFitCap: Partial<Record<"stu" | "bankq" | "paperq" | "mon", number>> = {};
let fittingPages = false;
let tasks: Array<Record<string, unknown>> = [];
let taskId = "";
let task: Record<string, unknown> | null = null;
let roster: Record<string, unknown> | null = null;
let monitor: Record<string, unknown> | null = null;
let results: Record<string, unknown> | null = null;
let analysis: Record<string, unknown> | null = null;
let monitorTimer: number | null = null;
let activeQid = "";
let showQForm = false;
let editQid = "";
let taskView: "current" | "history" = "current";
let analysisPass = "";
let analysisGood = "";
let analysisExcellent = "";
let analysisBands = "";
let analysisTopPct = "";
let analysisBottomPct = "";
let gradePane: "board" | "students" | "questions" = "board";
let gradeQ: Record<string, unknown> | null = null;
let gradeStu = "";
let gradePaper: Record<string, unknown> | null = null;

const root = document.getElementById("app")!;
const noticeHost = document.createElement("div");
noticeHost.id = "notice-host";
document.body.appendChild(noticeHost);

type NoticeKind = "ok" | "err" | "info";
let notice: { kind: NoticeKind; title: string; text: string } | null = null;

function tell(text: string, kind: NoticeKind = "info", title?: string): void {
  const titles: Record<NoticeKind, string> = { ok: "已完成", err: "无法完成", info: "提示" };
  notice = { kind, title: title || titles[kind], text };
  paintNotice();
}

function tellResult(ok: boolean, okText: string, data?: unknown, fallback?: string): void {
  if (ok) {
    tell(okText, "ok");
  } else {
    tell(errMsg(data, fallback), "err");
  }
}

function hideNotice(): void {
  notice = null;
  paintNotice();
}

function paintNotice(): void {
  if (!notice) {
    noticeHost.innerHTML = "";
    return;
  }
  const kicker = notice.kind === "err" ? "出错" : notice.kind === "ok" ? "完成" : "提示";
  noticeHost.innerHTML = `<div class="notice" data-notice>
    <div class="notice-box ${notice.kind}" role="alertdialog" aria-modal="true" aria-labelledby="notice-title">
      <div class="notice-kicker">${kicker}</div>
      <h2 id="notice-title">${esc(notice.title)}</h2>
      <p>${esc(notice.text)}</p>
      <p class="toolbar"><button type="button" class="primary" data-notice-ok>知道了</button></p>
    </div>
  </div>`;
  const ok = noticeHost.querySelector<HTMLButtonElement>("[data-notice-ok]");
  noticeHost.querySelector("[data-notice]")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) {
      hideNotice();
    }
  });
  ok?.addEventListener("click", hideNotice);
  ok?.focus();
}

async function tcall<T = Record<string, unknown>>(path: string, opts: RequestInit = {}) {
  const r = await api<T>(path, { ...opts, token });
  if (r.status === 401) {
    token = null;
    localStorage.removeItem(TOKEN_KEY);
    tell("登录已过期，请重新进入讲台", "err");
    render();
  }
  return r;
}

function bodyJson(v: unknown): RequestInit {
  return { method: "POST", body: JSON.stringify(v) };
}

function formFile(fd: FormData): File | null {
  const v = fd.get("file");
  return v instanceof File && v.size > 0 ? v : null;
}

function looksLikeRoster(text: string): boolean {
  const first = text.replace(/^\uFEFF/, "").split(/\r?\n/).find((line) => line.trim()) || "";
  return /班级|学号|姓名|class|student_no/i.test(first) || first.split(/[,\t;，]/).length >= 3;
}

async function readRosterCsv(file: File | null, pasted: string): Promise<string> {
  if (!file) {
    return pasted;
  }
  const lower = file.name.toLowerCase();
  if (/\.(xlsx|xls|xlsm)$/.test(lower)) {
    throw new Error("请把 Excel 另存为 CSV 后再导入，或直接粘贴三列：班级、学号、姓名");
  }
  const buf = await file.arrayBuffer();
  const utf8 = new TextDecoder("utf-8").decode(buf);
  if (looksLikeRoster(utf8) && !utf8.includes("\uFFFD")) {
    return utf8;
  }
  try {
    const gbk = new TextDecoder("gb18030").decode(buf);
    if (looksLikeRoster(gbk)) {
      return gbk;
    }
  } catch {
    /* some browsers do not ship gb18030 */
  }
  return utf8;
}

async function loadLan(): Promise<void> {
  const r = await tcall<{ addresses: string[]; port: number }>("/api/teacher/lan-addresses");
  if (r.status === 200) {
    lan = r.data;
  }
}

async function loadTab(): Promise<void> {
  if (!token) {
    return;
  }
  await loadLan();
  if (tab === "banks") {
    const r = await tcall<{ banks: Array<Record<string, unknown>> }>("/api/teacher/banks");
    if (r.status === 200) {
      banks = r.data.banks || [];
    }
    if (bankId) {
      const d = await tcall<Record<string, unknown>>(`/api/teacher/banks/${bankId}`);
      bank = d.status === 200 ? d.data : null;
      bankNeedPack = true;
      bankPackStarts = [0];
    }
  }
  if (tab === "roster") {
    const r = await tcall<{ students: Array<Record<string, unknown>> }>("/api/teacher/students");
    if (r.status === 200) {
      students = r.data.students || [];
    }
    const cr = await tcall<{ classes: Array<Record<string, unknown>> }>("/api/teacher/classes");
    if (cr.status === 200) {
      classes = cr.data.classes || [];
    }
  }
  if (tab === "papers") {
    const r = await tcall<{ papers: Array<Record<string, unknown>> }>("/api/teacher/papers");
    if (r.status === 200) {
      papers = r.data.papers || [];
    }
    const br = await tcall<{ banks: Array<Record<string, unknown>> }>("/api/teacher/banks");
    if (br.status === 200) {
      banks = br.data.banks || [];
    }
    if (paperId) {
      await loadPaperPreview();
    } else {
      paperQs = [];
      paperPreviewErr = "";
    }
  }
  if (tab === "tasks" || tab === "grades" || tab === "marking") {
    const r = await tcall<{ tasks: Array<Record<string, unknown>> }>("/api/teacher/tasks");
    if (r.status === 200) {
      tasks = r.data.tasks || [];
    }
    const br = await tcall<{ banks: Array<Record<string, unknown>> }>("/api/teacher/banks");
    if (br.status === 200) {
      banks = br.data.banks || [];
    }
    const sr = await tcall<{ students: Array<Record<string, unknown>> }>("/api/teacher/students");
    if (sr.status === 200) {
      students = sr.data.students || [];
    }
    const pr = await tcall<{ papers: Array<Record<string, unknown>> }>("/api/teacher/papers");
    if (pr.status === 200) {
      papers = pr.data.papers || [];
    }
    const cr = await tcall<{ classes: Array<Record<string, unknown>> }>("/api/teacher/classes");
    if (cr.status === 200) {
      classes = cr.data.classes || [];
    }
    if (taskId) {
      await loadTask();
    }
  }
}

async function saveEarly(on: boolean): Promise<void> {
  if (!taskId) {
    return;
  }
  const r = await tcall(`/api/teacher/tasks/${taskId}`, {
    method: "PATCH",
    body: JSON.stringify({ allow_early_submit: on }),
  });
  if (r.status === 204 && task) {
    task.allow_early_submit = on;
  } else {
    tell(errMsg(r.data), "err");
  }
}

function confirmAutoGrade(): boolean {
  return window.confirm(AUTO_GRADE_WARN);
}

async function saveAutoGrade(on: boolean): Promise<void> {
  if (!taskId) {
    return;
  }
  if (on && !confirmAutoGrade()) {
    render();
    return;
  }
  const r = await tcall(`/api/teacher/tasks/${taskId}`, {
    method: "PATCH",
    body: JSON.stringify({ auto_grade: on }),
  });
  if (r.status === 204 && task) {
    task.auto_grade = on;
    await loadTask();
    render();
  } else {
    tell(errMsg(r.data), "err");
    render();
  }
}

function analysisQuery(): string {
  const p = new URLSearchParams();
  if (analysisPass) p.set("pass_score", analysisPass);
  if (analysisGood) p.set("good_score", analysisGood);
  if (analysisExcellent) p.set("excellent_score", analysisExcellent);
  if (analysisBands) p.set("bands", analysisBands);
  if (analysisTopPct) p.set("top_pct", analysisTopPct);
  if (analysisBottomPct) p.set("bottom_pct", analysisBottomPct);
  const s = p.toString();
  return s ? `?${s}` : "";
}

async function loadTask(): Promise<void> {
  const t = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}`);
  task = t.status === 200 ? t.data : null;
  const ro = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/roster`);
  roster = ro.status === 200 ? ro.data : null;
  if (tab === "tasks") {
    const m = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/monitor`);
    monitor = m.status === 200 ? m.data : null;
  }
  if (tab === "grades" || tab === "marking") {
    const rs = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/results`);
    results = rs.status === 200 ? rs.data : null;
  }
  if (tab === "grades") {
    const an = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/analysis${analysisQuery()}`);
    analysis = an.status === 200 ? an.data : null;
  }
}

async function loadPaperPreview(): Promise<void> {
  paperQs = [];
  paperPreviewErr = "";
  paperNeedPack = true;
  paperPackStarts = [0];
  paperQPage = 1;
  if (!paperId) {
    return;
  }
  const r = await tcall<{ questions?: Array<Record<string, unknown>> }>(`/api/teacher/papers/${paperId}/preview`);
  if (r.status === 200) {
    paperQs = r.data.questions || [];
  } else {
    paperPreviewErr = errMsg(r.data);
  }
}

function studentUrls(): string[] {
  const port = lan?.port || 8080;
  const q = taskId ? `?t=${encodeURIComponent(taskId)}` : "";
  return (lan?.addresses || []).map((ip) => `http://${ip}:${port}/student.html${q}`);
}

async function downloadAuth(path: string, filename: string): Promise<void> {
  const res = await fetch(path, { headers: { authorization: `Bearer ${token}` } });
  const blob = await res.blob();
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
}

function setStuPicked(id: string, on: boolean): void {
  if (!id) {
    return;
  }
  if (on) {
    selectedStu.add(id);
  } else {
    selectedStu.delete(id);
  }
}

function paintStuPick(): void {
  const n = selectedStu.size;
  const filtered = students.filter((s) => !classFilter || String(s.class_name) === classFilter);
  const hint = root.querySelector("[data-stu-hint]");
  if (hint) {
    hint.textContent = `${classFilter ? `当前班级 ${classFilter} · ` : ""}共 ${filtered.length} 人${n ? ` · 已选 ${n}` : ""}`;
  }
  const del = root.querySelector<HTMLButtonElement>("[data-del-stus]");
  if (del) {
    del.hidden = n === 0;
    del.textContent = `删除所选（${n}）`;
  }
  const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-stu-row]"));
  const pageIds = rows.map((el) => el.dataset.stuRow || "");
  const master = root.querySelector<HTMLInputElement>("[data-pick-page]");
  if (master) {
    master.checked = pageIds.length > 0 && pageIds.every((id) => selectedStu.has(id));
  }
  rows.forEach((el) => {
    const id = el.dataset.stuRow || "";
    const on = selectedStu.has(id);
    el.classList.toggle("on", on);
    const box = el.querySelector("input");
    if (box) {
      box.checked = on;
    }
  });
}

function bind(): void {
  root.querySelectorAll("[data-tab]").forEach((el) => {
    el.addEventListener("click", async () => {
      tab = (el as HTMLElement).dataset.tab as Tab;
      const jump = (el as HTMLElement).dataset.task;
      if (jump) {
        taskId = jump;
      }
      await loadTab();
      render();
    });
  });
  root.querySelector("#logout")?.addEventListener("click", () => {
    token = null;
    localStorage.removeItem(TOKEN_KEY);
    render();
  });
  root.querySelectorAll("form").forEach((form) => {
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      void onSubmit(form);
    });
  });
  root.querySelectorAll("[data-bank]").forEach((el) => {
    el.addEventListener("click", async () => {
      bankId = (el as HTMLElement).dataset.bank || "";
      bankQPage = 1;
      showQForm = false;
      editQid = "";
      await loadTab();
      render();
    });
  });
  root.querySelectorAll("[data-task]").forEach((el) => {
    el.addEventListener("click", async () => {
      taskId = (el as HTMLElement).dataset.task || "";
      monPage = 1;
      gradePane = "board";
      gradeQ = null;
      analysisPass = "";
      analysisGood = "";
      analysisExcellent = "";
      analysisBands = "";
      analysisTopPct = "";
      analysisBottomPct = "";
      gradeStu = "";
      gradePaper = null;
      await loadTask();
      render();
    });
  });
  root.querySelectorAll("[data-class]").forEach((el) => {
    el.addEventListener("click", async () => {
      const name = (el as HTMLElement).dataset.class || "";
      classFilter = classFilter === name ? "" : name;
      selectedStu.clear();
      rosterPage = 1;
      render();
    });
  });
  root.querySelectorAll("[data-stu-row]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).closest("input")) {
        return;
      }
      const id = (el as HTMLElement).dataset.stuRow || "";
      setStuPicked(id, !selectedStu.has(id));
      paintStuPick();
    });
    const box = el.querySelector("input");
    box?.addEventListener("click", (ev) => ev.stopPropagation());
    box?.addEventListener("change", (ev) => {
      const id = (el as HTMLElement).dataset.stuRow || "";
      setStuPicked(id, (ev.currentTarget as HTMLInputElement).checked);
      paintStuPick();
    });
  });
  root.querySelector("[data-pick-page]")?.addEventListener("change", (ev) => {
    const on = (ev.currentTarget as HTMLInputElement).checked;
    root.querySelectorAll<HTMLElement>("[data-stu-row]").forEach((row) => {
      setStuPicked(row.dataset.stuRow || "", on);
    });
    paintStuPick();
  });
  root.querySelector("[data-del-stus]")?.addEventListener("click", () => {
    void act("del-stus");
  });
  root.querySelectorAll("[data-click]").forEach((el) => {
    el.addEventListener("click", () => {
      const [kind, id] = ((el as HTMLElement).dataset.click || "").split(":");
      void act(kind, id);
    });
  });
  root.querySelectorAll("[data-toggle]").forEach((el) => {
    el.addEventListener("click", () => {
      const name = (el as HTMLElement).dataset.toggle || "";
      const scope = el.closest("form") || root;
      const boxes = Array.from(scope.querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)).filter(
        (b) => b.offsetParent !== null,
      );
      const allOn = boxes.length > 0 && boxes.every((b) => b.checked);
      boxes.forEach((b) => {
        b.checked = !allOn;
      });
    });
  });
  root.querySelectorAll("[data-copy]").forEach((el) => {
    el.addEventListener("click", async () => {
      const url = (el as HTMLElement).dataset.copy || "";
      try {
        await navigator.clipboard.writeText(url);
        tell("答卷入口已复制", "ok");
      } catch {
        tell(url, "info", "请手动复制入口");
      }
    });
  });
  const filter = root.querySelector<HTMLInputElement>("#stu-filter");
  filter?.addEventListener("input", () => {
    const q = filter.value.trim().toLowerCase();
    root.querySelectorAll<HTMLElement>("[data-stu]").forEach((row) => {
      row.style.display = !q || (row.dataset.stu || "").includes(q) ? "" : "none";
    });
    syncPickAll();
  });
  root.querySelectorAll<HTMLInputElement>("[data-pick-visible]").forEach((master) => {
    master.addEventListener("change", () => {
      const name = master.dataset.pickVisible || "";
      const scope = master.closest("form") || root;
      const on = master.checked;
      visibleNamedBoxes(scope, name).forEach((b) => {
        b.checked = on;
      });
    });
  });
  root.querySelectorAll(".checks input[name]").forEach((el) => {
    el.addEventListener("change", syncPickAll);
  });
  syncPickAll();
  const mode = root.querySelector<HTMLSelectElement>("select[name=mode]");
  mode?.addEventListener("change", () => {
    const box = root.querySelector<HTMLInputElement>("form[name=task-create] input[name=allow_early_submit]");
    if (box) {
      box.checked = mode.value === "practice";
    }
  });
  root.querySelector<HTMLInputElement>("[data-early]")?.addEventListener("change", (ev) => {
    const box = ev.currentTarget as HTMLInputElement;
    void saveEarly(box.checked);
  });
  root.querySelectorAll<HTMLInputElement>("[data-auto-grade]").forEach((box) => {
    box.addEventListener("change", () => {
      void saveAutoGrade(box.checked);
    });
  });
  root.querySelectorAll<HTMLInputElement>("form[name=paper-create] input[name=auto_grade], form[name=task-create] input[name=auto_grade]").forEach((box) => {
    box.addEventListener("change", () => {
      if (box.checked && !confirmAutoGrade()) {
        box.checked = false;
      }
    });
  });
  const paperSel = root.querySelector<HTMLSelectElement>("form[name=task-create] select[name=paper_id]");
  const taskAuto = root.querySelector<HTMLInputElement>("form[name=task-create] input[name=auto_grade]");
  paperSel?.addEventListener("change", () => {
    if (!taskAuto) {
      return;
    }
    const p = papers.find((x) => String(x.id) === paperSel.value);
    taskAuto.checked = p ? p.auto_grade !== false : true;
  });
  bindPaperScores();
  root.querySelectorAll("[data-qid]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).closest("button")) {
        return;
      }
      const id = (el as HTMLElement).dataset.qid || "";
      const next = activeQid === id ? "" : id;
      activeQid = next;
      root.querySelectorAll(".qitem").forEach((n) => n.classList.toggle("on", (n as HTMLElement).dataset.qid === next));
    });
  });
  root.querySelectorAll("[data-edit-q]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      editQid = (el as HTMLElement).dataset.editQ || "";
      activeQid = editQid;
      showQForm = true;
      render();
    });
  });
  root.querySelector("[data-add-q]")?.addEventListener("click", () => {
    editQid = "";
    showQForm = true;
    render();
  });
  root.querySelector("[data-cancel-q]")?.addEventListener("click", () => {
    showQForm = false;
    editQid = "";
    render();
  });
  root.querySelectorAll("[data-task-view]").forEach((el) => {
    el.addEventListener("click", () => {
      taskView = ((el as HTMLElement).dataset.taskView || "current") as "current" | "history";
      if (taskId) {
        const cur = tasks.find((t) => t.id === taskId);
        if (cur && isHistory(cur.status) !== (taskView === "history")) {
          taskId = "";
          task = null;
        }
      }
      render();
    });
  });
  root.querySelectorAll("[data-paper]").forEach((el) => {
    el.addEventListener("click", async () => {
      paperId = (el as HTMLElement).dataset.paper || "";
      paperQPage = 1;
      await loadPaperPreview();
      render();
    });
  });
  root.querySelector("[data-paper-new]")?.addEventListener("click", () => {
    showPaperForm = true;
    render();
  });
  root.querySelector("[data-task-new]")?.addEventListener("click", () => {
    showTaskForm = true;
    render();
  });
  root.querySelector("[data-modal]")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) {
      showPaperForm = false;
      showTaskForm = false;
      render();
    }
  });
  root.querySelector("[data-close-modal]")?.addEventListener("click", () => {
    showPaperForm = false;
    showTaskForm = false;
    render();
  });
  root.querySelectorAll("[data-page]").forEach((el) => {
    el.addEventListener("click", () => {
      if ((el as HTMLButtonElement).disabled) {
        return;
      }
      const [which, dir] = ((el as HTMLElement).dataset.page || "").split(":");
      const delta = dir === "next" ? 1 : -1;
      if (which === "stu") {
        rosterPage = Math.max(1, rosterPage + delta);
      } else if (which === "bankq") {
        bankQPage = Math.max(1, bankQPage + delta);
      } else if (which === "paperq") {
        paperQPage = Math.max(1, paperQPage + delta);
      } else if (which === "mon") {
        monPage = Math.max(1, monPage + delta);
      }
      render();
    });
  });
  root.querySelectorAll("[data-grade-pane]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const pane = ((el as HTMLElement).dataset.gradePane || "board") as typeof gradePane;
      if (pane === "board" || pane === "students" || pane === "questions") {
        gradePane = pane;
        render();
      }
    });
  });
  root.querySelectorAll("[data-grade-q]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const idx = Number((el as HTMLElement).dataset.gradeQ);
      const qs = (analysis?.questions as Array<Record<string, unknown>>) || [];
      gradeQ = qs.find((q) => Number(q.index) === idx) || null;
      render();
    });
  });
  root.querySelector(".modal[data-grade-q-close]")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) {
      gradeQ = null;
      render();
    }
  });
  root.querySelector("button[data-grade-q-close]")?.addEventListener("click", () => {
    gradeQ = null;
    render();
  });
  root.querySelectorAll("[data-grade-stu]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const id = (el as HTMLElement).dataset.gradeStu || "";
      gradeStu = tab === "marking" ? id : gradeStu === id ? "" : id;
      if (!gradeStu || String(gradePaper?.attempt_id || "") !== gradeStu) {
        gradePaper = null;
      }
      render();
    });
  });
  root.querySelectorAll("[data-score-n]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const btn = el as HTMLButtonElement;
      if (btn.disabled) {
        return;
      }
      void saveOneCodeScore(
        btn.dataset.attempt || "",
        btn.dataset.qid || "",
        Number(btn.dataset.scoreN),
        Number(btn.dataset.max || 0),
      );
    });
  });
  root.querySelectorAll<HTMLInputElement>("[data-score-input]").forEach((inp) => {
    inp.addEventListener("change", () => {
      if (inp.disabled || inp.value === "") {
        return;
      }
      void saveOneCodeScore(inp.dataset.attempt || "", inp.dataset.qid || "", Number(inp.value), Number(inp.max || 0));
    });
  });
  root.querySelector("[data-mark-next]")?.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const id = (ev.currentTarget as HTMLElement).dataset.markNext || "";
    if (id) {
      gradeStu = id;
      gradePaper = null;
      render();
    }
  });
  root.querySelector(".modal[data-grade-paper-close]")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) {
      gradePaper = null;
      render();
    }
  });
  root.querySelector("button[data-grade-paper-close]")?.addEventListener("click", () => {
    gradePaper = null;
    render();
  });
  root.querySelector<HTMLSelectElement>("form[name=question] [name=qtype]")?.addEventListener("change", (ev) => {
    const form = (ev.currentTarget as HTMLElement).closest("form");
    if (form) {
      syncQuestionTypeFields(form);
    }
  });
  fillQuestionForm();
}

async function onSubmit(form: HTMLFormElement): Promise<void> {
  const fd = new FormData(form);
  const name = form.getAttribute("name");
  const val = (k: string) => String(fd.get(k) || "").trim();
  try {
    if (name === "gate") {
      const password = val("password");
      let r = await api<{ token?: string; error?: { code?: string } }>("/api/teacher/login", bodyJson({ password }));
      if (r.status === 409 && (r.data as { error?: { code?: string } }).error?.code === "SETUP_REQUIRED") {
        r = await api("/api/teacher/setup", bodyJson({ password }));
      }
      if (r.status >= 400) {
        tell(errMsg(r.data, "口令校验失败"), "err");
        return;
      }
      token = (r.data as { token: string }).token;
      localStorage.setItem(TOKEN_KEY, token);
      tab = "banks";
      await loadTab();
      render();
      return;
    }
    if (!token) {
      return;
    }
    if (name === "password") {
      const r = await tcall("/api/teacher/password", {
        method: "PUT",
        body: JSON.stringify({
          current_password: val("current_password"),
          new_password: val("new_password"),
        }),
      });
      tellResult(r.status === 204, "口令已更新", r.data);
    }
    if (name === "bank-create") {
      const r = await tcall("/api/teacher/banks", bodyJson({ name: val("name") }));
      tellResult(r.status === 201, "题库已创建", r.data);
    }
    if (name === "bank-import") {
      const file = formFile(fd);
      const html = file ? await file.text() : "";
      if (!html.trim()) {
        tell("请先选择 HTML 题库文件", "err");
        return;
      }
      const r = await tcall(`/api/teacher/banks/${bankId}/import-html`, {
        method: "POST",
        body: JSON.stringify({ filename: file?.name || "upload.html", html }),
      });
      tellResult(
        r.status < 400,
        `导入成功 ${(r.data as { imported?: number }).imported || 0} 题，失败 ${(r.data as { failed?: number }).failed || 0}`,
        r.data,
      );
    }
    if (name === "question") {
      const qtype = val("qtype");
      const choice = qtype === "single" || qtype === "multiple";
      const options = choice
        ? String(fd.get("options") || "")
            .split("\n")
            .map((line) => line.trim())
            .filter(Boolean)
            .map((line) => {
              const [key, ...rest] = line.split("|");
              return { key: key.trim(), text: rest.join("|").trim() || key.trim() };
            })
        : [];
      const rawAns =
        qtype === "judge"
          ? val("answer_judge")
          : qtype === "fill"
            ? val("answer_fill")
            : qtype === "code"
              ? val("answer_code")
              : val("answer");
      let answer: unknown = { kind: qtype, value: rawAns };
      if (qtype === "multiple") {
        answer = { kind: "multiple", value: rawAns.split(/[,，\s]+/).filter(Boolean) };
      } else if (qtype === "judge") {
        answer = { kind: "judge", value: rawAns === "true" || rawAns === "对" };
      } else if (qtype === "fill") {
        answer = { kind: "fill", value: rawAns.split("|").map((s) => s.trim()).filter(Boolean) };
      }
      const payload = { qtype, stem: val("stem"), options, answer, source: val("source") };
      const qid = val("qid");
      const r = qid
        ? await tcall(`/api/teacher/questions/${qid}`, { method: "PUT", body: JSON.stringify(payload) })
        : await tcall(`/api/teacher/banks/${bankId}/questions`, bodyJson(payload));
      tellResult(r.status < 400, "题目已保存", r.data);
      if (r.status < 400) {
        showQForm = false;
        editQid = "";
      }
    }
    if (name === "csv") {
      const file = formFile(fd);
      let csv = "";
      try {
        csv = (await readRosterCsv(file, val("csv"))).trim();
      } catch (e) {
        tell(e instanceof Error ? e.message : String(e), "err");
        return;
      }
      if (!csv) {
        tell("请先选择 CSV 文件，或粘贴班级、学号、姓名", "err");
        return;
      }
      const r = await tcall("/api/teacher/students/import-csv", {
        method: "POST",
        body: JSON.stringify({ filename: file?.name || "students.csv", csv }),
      });
      if (r.status < 400) {
        const data = r.data as {
          upserted?: number;
          failed?: number;
          failures?: Array<{ line?: number; reason?: string }>;
        };
        const extra = (data.failures || [])
          .slice(0, 6)
          .map((f) => `第 ${f.line} 行：${f.reason}`)
          .join("\n");
        tell(`名册写入 ${data.upserted || 0} 人，跳过 ${data.failed || 0} 行` + (extra ? `\n${extra}` : ""), extra ? "info" : "ok");
      } else {
        tell(errMsg(r.data), "err");
      }
    }
    if (name === "class-create") {
      const r = await tcall("/api/teacher/classes", bodyJson({ name: val("name") }));
      tellResult(r.status === 201, "班级已创建", r.data);
    }
    if (name === "student-create") {
      const r = await tcall("/api/teacher/students", bodyJson({
        class_name: val("class_name"),
        student_no: val("student_no"),
        name: val("name"),
      }));
      tellResult(r.status === 201, "学生已加入名册", r.data);
    }
    if (name === "paper-create") {
      const type_counts: Record<string, number> = {};
      for (const [k] of QTYPES) {
        const n = Number(fd.get(`count_${k}`) || 0);
        if (n > 0) {
          type_counts[k] = n;
        }
      }
      const bank_ids = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=bank_id]:checked")).map((i) => i.value);
      if (!bank_ids.length) {
        tell("请先勾选题库", "err");
        return;
      }
      if (!Object.keys(type_counts).length) {
        tell("请填写至少一种题型的题量", "err");
        return;
      }
      const bank_draws = bank_ids
        .map((id) => {
          const count = Number(fd.get(`draw_count_${id}`) || 0);
          const ratio = Number(fd.get(`draw_ratio_${id}`) || 0);
          const d: { bank_id: string; count?: number; ratio?: number } = { bank_id: id };
          if (count > 0) {
            d.count = count;
          } else if (ratio > 0) {
            d.ratio = ratio;
          }
          return d;
        })
        .filter((d) => d.count || d.ratio);
      if (bank_draws.length && bank_draws.length !== bank_ids.length) {
        tell("请为每个勾选的题库填写抽题数量或比例，或全部留空", "err");
        return;
      }
      const r = await tcall("/api/teacher/papers", {
        method: "POST",
        body: JSON.stringify({
          title: val("title"),
          total_score: Number(val("total_score") || 100),
          pass_score: val("pass_score") ? Number(val("pass_score")) : undefined,
          rule: {
            bank_ids,
            type_counts,
            shuffle_within_section: fd.get("shuffle_within_section") === "on",
            shuffle_options: fd.get("shuffle_options") === "on",
            bank_draws,
          },
          live_draw: fd.get("live_draw") === "on",
          auto_grade: fd.get("auto_grade") === "on",
        }),
      });
      tellResult(r.status === 201, "试卷已保存，可在任务里选用", r.data);
      if (r.status === 201) {
        paperId = String((r.data as { id: string }).id);
        showPaperForm = false;
        paperQPage = 1;
      }
    }
    if (name === "task-create") {
      const class_names = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=class_name]:checked")).map(
        (i) => i.value,
      );
      const peopleOpen = Boolean(form.querySelector<HTMLDetailsElement>("details.fold")?.open);
      const student_ids =
        class_names.length && !peopleOpen
          ? []
          : Array.from(form.querySelectorAll<HTMLInputElement>("input[name=student_id]:checked")).map((i) => i.value);
      const paper_id = val("paper_id");
      if (!paper_id) {
        tell("请先在试卷管理中组卷，再选择试卷", "err");
        return;
      }
      const r = await tcall("/api/teacher/tasks", {
        method: "POST",
        body: JSON.stringify({
          title: val("title"),
          mode: val("mode"),
          duration_minutes: Number(val("duration_minutes") || 60),
          allow_early_submit: fd.get("allow_early_submit") === "on",
          student_ids,
          class_names,
          paper_id,
          auto_grade: fd.get("auto_grade") === "on",
        }),
      });
      if (r.status === 201) {
        taskId = String((r.data as { id: string }).id);
        tell("任务已生成，可以发布", "ok");
        taskView = "current";
        showTaskForm = false;
      } else if (r.status === 409) {
        const details = (r.data as { error?: { details?: { task_id?: string }; message?: string } }).error;
        taskId = details?.details?.task_id || taskId;
        tell(details?.message || errMsg(r.data), "err");
        showTaskForm = false;
        taskView = "current";
      } else {
        tell(errMsg(r.data), "err");
      }
    }
    if (name === "roster") {
      if (!task || String(task.status) !== "draft") {
        tell("发布后名单已冻结", "err");
        return;
      }
      const student_ids = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=student_id]:checked")).map(
        (i) => i.value,
      );
      const r = await tcall(`/api/teacher/tasks/${taskId}/roster`, {
        method: "PUT",
        body: JSON.stringify({ student_ids }),
      });
      tellResult(r.status < 400, "名单已更新", r.data);
    }
    if (name === "early") {
      const r = await tcall(`/api/teacher/tasks/${taskId}`, {
        method: "PATCH",
        body: JSON.stringify({ allow_early_submit: fd.get("allow_early_submit") === "on" }),
      });
      if (r.status !== 204) {
        tell(errMsg(r.data), "err");
      }
    }
    if (name === "code-score") {
      const attempt = val("attempt_id");
      const scores: Record<string, number> = {};
      form.querySelectorAll<HTMLInputElement>("input[data-qid]").forEach((inp) => {
        if (inp.value === "") {
          return;
        }
        const qid = inp.dataset.qid || "";
        if (qid) {
          scores[qid] = clampScore(Number(inp.value), Number(inp.max || 0));
        }
      });
      const r = await tcall(`/api/teacher/attempts/${attempt}/code-scores`, {
        method: "PUT",
        body: JSON.stringify({ scores }),
      });
      tellResult(r.status < 400, "编程题分数已写入", r.data);
    }
    if (name === "analysis") {
      analysisPass = val("pass_score");
      analysisGood = val("good_score");
      analysisExcellent = val("excellent_score");
      analysisTopPct = val("top_pct");
      analysisBottomPct = val("bottom_pct");
      const cuts = [val("band1"), val("band2"), val("band3"), val("band4"), val("band5")];
      analysisBands = cuts.every((x) => x) ? cuts.join(",") : "";
      const r = await tcall(`/api/teacher/tasks/${taskId}/analysis${analysisQuery()}`);
      analysis = r.status === 200 ? r.data : analysis;
      tellResult(r.status === 200, "分析已刷新", r.data);
    }
    await loadTab();
    render();
  } catch (e) {
    tell(String(e), "err");
  }
}

async function act(kind: string, id?: string): Promise<void> {
  const confirms: Record<string, string> = {
    publish: "确认发布？名单内学生即可进入考场。",
    close: "确认收卷？正在作答的学生将被提交，任务继续进行。",
    end: "确认结束任务？未交卷学生将无法继续，任务转入历史后才能公开试卷。",
    "confirm-mark": "确认该生批改结果？",
    release: "确认公开试卷？需已结束任务，且全部有编程题的学生都已确认批改。学生机将看到成绩与解析。",
    "del-bank": "确认删除该题库？",
    "del-q": "确认删除这道题？",
    "del-paper": "确认删除这份试卷？",
    "del-class": "确认删除该班级？班内有学生时不能删。",
    "del-stu": "确认从名册中删除该学生？",
    "del-stus": "确认删除所选学生？",
    "del-task": "确认删除该任务？已拆出的子任务会保留。",
    split: "按班级拆成子任务？每个班会得到独立入口，原任务将删除。",
  };
  if (kind === "del-stus" && selectedStu.size === 0) {
    tell("请先勾选要删除的学生", "err");
    return;
  }
  if (confirms[kind] && !window.confirm(confirms[kind])) {
    return;
  }
  if ((kind === "publish" || kind === "close" || kind === "end" || kind === "split") && task && isHistory(task.status)) {
    tell("历史任务不能再发布、收卷、结束或拆分子任务", "err");
    return;
  }
  if (kind === "split" && task && task.parent_id) {
    tell("子任务不能再拆分", "err");
    return;
  }
  if (kind === "split" && tasks.some((t) => String(t.parent_id) === id)) {
    tell("已经拆过，请打开各班子任务", "err");
    return;
  }
  if (kind === "publish" && tasks.some((t) => String(t.parent_id) === id)) {
    tell("原任务已拆分，请发布各班子任务", "err");
    return;
  }
  if (kind === "release") {
    const status = String(task?.status || "");
    if (status === "published" || status === "draft") {
      tell("结束后才能公开试卷", "err");
      return;
    }
    if (status !== "closed" && status !== "graded") {
      tell("批改完成后才能公开试卷", "err");
      return;
    }
    if (markingRows().some(markingPending)) {
      tell("还有学生未确认批改", "err");
      return;
    }
  }
  const map: Record<string, [string, string]> = {
    publish: ["POST", `/api/teacher/tasks/${id}/publish`],
    close: ["POST", `/api/teacher/tasks/${id}/close`],
    end: ["POST", `/api/teacher/tasks/${id}/end`],
    grade: ["POST", `/api/teacher/tasks/${id}/grade`],
    release: ["POST", `/api/teacher/tasks/${id}/release`],
  };
  if (kind === "del-bank") {
    const r = await tcall(`/api/teacher/banks/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "题库已删除", r.data);
    bankId = "";
  } else if (kind === "del-q") {
    const r = await tcall(`/api/teacher/questions/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "题目已删除", r.data);
    if (activeQid === id) {
      activeQid = "";
    }
    if (editQid === id) {
      editQid = "";
      showQForm = false;
    }
  } else if (kind === "del-paper") {
    const r = await tcall(`/api/teacher/papers/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "试卷已删除", r.data);
    paperId = paperId === id ? "" : paperId;
    if (!paperId) {
      paperQs = [];
      paperPreviewErr = "";
    }
  } else if (kind === "del-class") {
    const r = await tcall(`/api/teacher/classes/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "班级已删除", r.data);
    if (r.status === 204) {
      const gone = classes.find((c) => String(c.id) === id);
      if (gone && classFilter === gone.name) {
        classFilter = "";
      }
      selectedStu.clear();
    }
  } else if (kind === "del-stus") {
    const ids = Array.from(selectedStu);
    let ok = 0;
    let fail = "";
    for (const sid of ids) {
      const r = await tcall(`/api/teacher/students/${sid}`, { method: "DELETE" });
      if (r.status === 204) {
        ok += 1;
        selectedStu.delete(sid);
      } else {
        fail = errMsg(r.data);
        break;
      }
    }
    if (ok && !fail) {
      tell(`已删除 ${ok} 名学生`, "ok");
    } else if (ok) {
      tell(`已删除 ${ok} 人，后续失败：${fail}`, "err");
    } else {
      tell(fail || "删除失败", "err");
    }
  } else if (kind === "del-stu") {
    const r = await tcall(`/api/teacher/students/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "学生已删除", r.data);
  } else if (kind === "del-task") {
    const r = await tcall(`/api/teacher/tasks/${id}`, { method: "DELETE" });
    tellResult(r.status === 204, "任务已删除", r.data);
    if (r.status === 204 && taskId === id) {
      taskId = "";
      task = null;
    }
  } else if (kind === "split") {
    const r = await tcall(`/api/teacher/tasks/${id}/split`, { method: "POST", body: "{}" });
    if (r.status < 400) {
      const kids = ((r.data as { tasks?: Array<{ id?: string }> }).tasks || []);
      tell(`已按班级拆成 ${kids.length} 个子任务，原任务已删除`, "ok");
      if (kids[0]?.id) {
        taskId = String(kids[0].id);
      }
    } else {
      tell(errMsg(r.data), "err");
    }
  } else if (kind === "tpl") {
    await downloadAuth("/api/teacher/student-template.csv", "students-template.csv");
    return;
  } else if (kind === "stu-csv") {
    await downloadAuth("/api/teacher/students.csv", "students.csv");
    return;
  } else if (kind === "export") {
    const row = banks.find((b) => String(b.id) === id);
    const name = String(row?.name || "题库").replace(/[\\/:*?"<>|]/g, "_");
    await downloadAuth(`/api/teacher/banks/${id}/export-html`, `${name}.html`);
    return;
  } else if (kind === "csv") {
    const res = await fetch(`/api/teacher/tasks/${id}/results.csv`, {
      headers: { authorization: `Bearer ${token}` },
    });
    const blob = await res.blob();
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    const title = String(task?.title || "成绩").replace(/[\\/:*?"<>|]/g, "_");
    a.download = `${title}-成绩.csv`;
    a.click();
    return;
  } else if (kind === "view-paper") {
    const r = await tcall<Record<string, unknown>>(`/api/teacher/attempts/${id}`);
    if (r.status === 200) {
      gradePaper = r.data;
    } else {
      tell(errMsg(r.data), "err");
    }
    render();
    return;
  } else if (kind === "confirm-mark") {
    const r = await tcall(`/api/teacher/attempts/${id}/confirm-marking`, {
      method: "POST",
      body: "{}",
    });
    if (r.status < 400) {
      const row = markingRows().find((x) => String(x.attempt_id) === id);
      if (row) {
        row.marking_confirmed = true;
      }
      const next = nextPendingAttempt(id || "");
      if (next) {
        gradeStu = next;
        gradePaper = null;
      }
      tell("该生批改已确认", "ok");
    } else {
      tell(errMsg(r.data), "err");
    }
  } else if (map[kind]) {
    const [method, path] = map[kind];
    const r = await tcall(path, { method, body: method === "POST" ? "{}" : undefined });
    tellResult(r.status < 400, "操作完成", r.data);
  }
  await loadTab();
  render();
}

function fillQuestionForm(): void {
  const form = root.querySelector<HTMLFormElement>("form[name=question]");
  if (!form || !showQForm) {
    return;
  }
  const qid = form.querySelector<HTMLInputElement>("[name=qid]");
  if (qid) {
    qid.value = editQid;
  }
  if (!editQid) {
    syncQuestionTypeFields(form);
    return;
  }
  const q = ((bank?.questions as Array<Record<string, unknown>>) || []).find((x) => String(x.id) === editQid);
  if (!q) {
    return;
  }
  const set = (name: string, v: string) => {
    const el = form.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>(`[name=${name}]`);
    if (el) {
      el.value = v;
    }
  };
  set("qtype", String(q.qtype || "single"));
  set("source", String(q.source || ""));
  set("stem", String(q.stem || ""));
  const opts = (q.options as Array<{ key?: string; text?: string }>) || [];
  set("options", opts.map((o) => `${o.key || ""}|${o.text || ""}`).join("\n"));
  const ans = answerToInput(q);
  set("answer", ans);
  set("answer_judge", ans === "true" ? "true" : "false");
  set("answer_fill", ans);
  set("answer_code", ans);
  syncQuestionTypeFields(form);
}

function syncQuestionTypeFields(form: HTMLFormElement): void {
  const qtype = form.querySelector<HTMLSelectElement>("[name=qtype]")?.value || "single";
  form.querySelectorAll<HTMLElement>(".q-options, .q-ans").forEach((el) => {
    const types = (el.dataset.for || "").split(",").filter(Boolean);
    const on = types.includes(qtype);
    el.hidden = !on;
    el.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>("input, textarea, select").forEach(
      (inp) => {
        inp.disabled = !on;
      },
    );
  });
}

function gateHtml(): string {
  return `<div class="gate">
    <div class="kicker">控制室</div>
    <h1>讲台口令</h1>
    <p class="hint">第一次进入会把口令写进本机数据库，之后用同一口令解锁。</p>
    <form name="gate">
      <label class="field">教师口令<input name="password" type="password" required autocomplete="current-password" /></label>
      <p class="toolbar"><button class="primary" type="submit">进入讲台</button></p>
    </form>
  </div>`;
}

function navHtml(): string {
  const items: [Tab, string][] = [
    ["banks", "题库管理"],
    ["roster", "班级名册"],
    ["papers", "试卷管理"],
    ["tasks", "任务管理"],
    ["marking", "试卷批改"],
    ["grades", "成绩分析"],
  ];
  return `<nav>${items
    .map(([k, lab]) => `<button type="button" data-tab="${k}" class="${tab === k ? "active" : ""}">${lab}</button>`)
    .join("")}</nav>`;
}

function titles(): Record<Tab, [string, string]> {
  return {
    banks: ["题库管理", "导入规范 HTML，或在本页增改删题目。"],
    roster: ["班级名册", "先建班级，再导入或逐条登记学生。"],
    papers: ["试卷管理", "点新建组卷；选中试卷后右侧预览题目。"],
    tasks: ["任务管理", "点新建任务；按钮随状态出现。发布后名单冻结，只监考。"],
    marking: ["试卷批改", "点色块即给分。给完该生编程题后点提交确认。全部确认且任务结束后可公开试卷。"],
    grades: ["成绩分析", "选任务看总览、学生成绩和题目分析。"],
  };
}

function visibleNamedBoxes(scope: ParentNode, name: string): HTMLInputElement[] {
  return Array.from(scope.querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)).filter(
    (b) => b.offsetParent !== null,
  );
}

function syncPickAll(): void {
  root.querySelectorAll<HTMLInputElement>("[data-pick-visible]").forEach((master) => {
    const name = master.dataset.pickVisible || "";
    const scope = master.closest("form") || root;
    const boxes = visibleNamedBoxes(scope, name);
    master.checked = boxes.length > 0 && boxes.every((b) => b.checked);
  });
}

function checkList(
  name: string,
  items: Array<{ id: string; label: string; checked?: boolean; extra?: string }>,
  pickAll?: string,
): string {
  const head = pickAll
    ? `<label class="check pick-all"><input type="checkbox" data-pick-visible="${esc(name)}" /> <span>${esc(pickAll)}</span></label>`
    : "";
  return `${head}<div class="checks">${items
    .map(
      (it) =>
        `<label class="check"${it.extra ? ` data-stu="${esc(it.extra.toLowerCase())}"` : ""}>
         <input type="checkbox" name="${esc(name)}" value="${esc(it.id)}" ${it.checked ? "checked" : ""} />
         <span>${esc(it.label)}</span>
       </label>`,
    )
    .join("")}</div>`;
}

function monitorInner(): string {
  const classHint = childClassName();
  const list = ((monitor?.students as Array<Record<string, unknown>>) || []).filter(
    (s) => !classHint || String(s.class_name) === classHint,
  );
  const n = list.length;
  const entered = list.filter((s) => s.status !== "not_entered").length;
  const doing = list.filter((s) => s.status === "in_progress").length;
  const done = list.filter((s) => s.status === "submitted").length;
  const mSize = pageSizeFor("mon");
  const mPages = Math.max(1, Math.ceil((list.length || 1) / mSize));
  if (monPage > mPages) {
    monPage = mPages;
  }
  const mFrom = list.length ? (monPage - 1) * mSize : 0;
  const rows = list
    .slice(mFrom, mFrom + mSize)
    .map(
      (s) =>
        `<tr><td>${esc(s.class_name)}</td><td class="mono">${esc(s.student_no)}</td><td>${esc(s.name)}</td>
         <td><span class="badge ${badgeFor(s.status)}">${esc(statusLabel(s.status))}</span></td></tr>`,
    )
    .join("");
  return `<div class="stats">
      <div class="stat"><b>${n}</b><span>应考</span></div>
      <div class="stat"><b>${entered}</b><span>已进场</span></div>
      <div class="stat"><b>${doing}</b><span>作答中</span></div>
      <div class="stat"><b>${done}</b><span>已交卷</span></div>
    </div>
    <div class="scroll" data-fit="mon"><table><thead><tr><th>班级</th><th>学号</th><th>姓名</th><th>状态</th></tr></thead><tbody>${rows || `<tr><td colspan="4" class="muted">暂无名单</td></tr>`}</tbody></table></div>
    ${pagerHtml("mon", monPage, mPages, list.length)}`;
}

function accessHtml(): string {
  const urls = studentUrls();
  const live = tasks.find((t) => t.status === "published");
  return `<div class="split">
    <section class="panel">
      <h2>答卷入口</h2>
      <p class="hint">学生机浏览器打开下面地址。教室断外网也不影响，字体已改为系统字体。</p>
      ${
        urls.length
          ? urls
              .map(
                (url) => `<div class="url-box lan">
                  <a href="${esc(url)}" target="_blank">${esc(url)}</a>
                  <button type="button" class="ghost" data-copy="${esc(url)}">复制</button>
                </div>`,
              )
              .join("")
          : "<p>暂无局域网地址，请检查网卡。</p>"
      }
      <form name="password" class="row">
        <label class="field">当前口令<input name="current_password" type="password" required /></label>
        <label class="field">新口令<input name="new_password" type="password" required /></label>
        <button class="primary" type="submit">改口令</button>
      </form>
    </section>
    <section class="panel">
      <h2>当场考试</h2>
      ${
        live
          ? `<p>${clipTitle(live.title)} · ${esc(modeLabel(live.mode))}
             <span class="badge ${badgeFor(live.status)}">${esc(statusLabel(live.status))}</span></p>
             <p class="hint">结束于 ${esc(live.closes_at || "—")}</p>
             <p class="toolbar"><button type="button" data-tab="tasks" data-task="${esc(live.id)}">进入监考</button></p>`
          : `<p class="hint">当前没有进行中的考试。去「组卷」生成任务并发布。</p>`
      }
    </section>
  </div>`;
}

function questionSheetHtml(
  questions: Array<Record<string, unknown>>,
  from = 0,
  count = questions.length,
  canEdit = true,
  scorePlan?: Record<string, number[]>,
): string {
  if (!questions.length) {
    return `<p class="hint">${canEdit ? "本题库还没有题目。从上方导入 HTML，或点「单题增加」。" : "这份试卷还没有可预览的题目。"}</p>`;
  }
  let last = "";
  let typeIndex = 0;
  const blocks: string[] = [];
  questions.forEach((q, i) => {
    const sec = typeSection(String(q.qtype));
    const typeChanged = sec !== last;
    if (typeChanged) {
      typeIndex += 1;
      last = sec;
    }
    if (i < from || i >= from + count) {
      return;
    }
    if (typeChanged || i === from) {
      const cap = !canEdit ? scoreCaptionHtml(String(q.qtype), scorePlan?.[String(q.qtype)] || []) : "";
      blocks.push(`<h3>${chineseIndex(typeIndex)}、${esc(sec)}${cap}</h3>`);
    }
    const opts = ((q.options as Array<{ key?: string; text?: string }>) || [])
      .map((o) => `${esc(o.key)}. ${esc(o.text)}`)
      .join("　");
    const on = String(q.id) === activeQid ? " on" : "";
    const stem = displayStemHtml(q.stem_html, q.stem, !!opts, `第${i + 1}题 `);
    blocks.push(`<article class="qitem${on}" tabindex="0" data-qid="${esc(q.id)}">
      <div class="question">
        ${stem}
        ${opts ? `<div class="options">${opts}</div>` : ""}
        <div class="answer">${formatAnswerHtml(q)}</div>
      </div>
      ${
        canEdit
          ? `<div class="qitem-ops">
        <button type="button" data-edit-q="${esc(q.id)}">修改</button>
        <button type="button" class="danger" data-click="del-q:${esc(q.id)}">删除</button>
      </div>`
          : ""
      }
    </article>`);
  });
  return `<div class="qsheet">${blocks.join("")}</div>`;
}

function questionFormHtml(): string {
  return `<form name="question" class="subpanel">
    <h3>${editQid ? "修改题目" : "单题增加"}</h3>
    <input type="hidden" name="qid" />
    <div class="row">
      <label class="field">题型<select name="qtype">${QTYPES.map(([k, l]) => `<option value="${k}">${l}</option>`).join("")}</select></label>
      <label class="field">来源<input name="source" /></label>
    </div>
    <label class="field">题干<textarea name="stem" required></textarea></label>
    <label class="field q-options" data-for="single,multiple">选项（每行 键|文本）<textarea name="options" placeholder="A|选项A"></textarea></label>
    <label class="field q-ans" data-for="single,multiple">答案<input name="answer" placeholder="单选填 A，多选填 A,C" /></label>
    <label class="field q-ans" data-for="judge">答案<select name="answer_judge"><option value="true">对</option><option value="false">错</option></select></label>
    <label class="field q-ans" data-for="fill">答案<input name="answer_fill" placeholder="多空用 | 分隔" /></label>
    <label class="field q-ans" data-for="code">参考答案<textarea name="answer_code" rows="8" placeholder="可选，供批改对照"></textarea></label>
    <p class="toolbar">
      <button class="primary" type="submit">保存题目</button>
      <button type="button" class="ghost" data-cancel-q>取消</button>
    </p>
  </form>`;
}

function banksHtml(): string {
  const list = banks
    .map(
      (b) => `<tr class="pick${bankId === b.id ? " on" : ""}" data-bank="${esc(b.id)}">
      <td>${esc(b.name)}</td>
      <td>${esc(b.question_count)}</td>
    </tr>`,
    )
    .join("");
  const qs = (bank?.questions as Array<Record<string, unknown>>) || [];
  const bankName = bank ? String((bank as { name?: string }).name || "") : "";
  const qSlice = questionSlice(bankNeedPack, bankPackStarts, bankQPage, qs.length);
  if (bankQPage > qSlice.pages) {
    bankQPage = qSlice.pages;
  }
  return `<div class="grid">
    <section class="panel">
      <h2>题库列表</h2>
      <form name="bank-create" class="row">
        <label class="field">名称<input name="name" required /></label>
        <button class="primary" type="submit">新建</button>
      </form>
      <div class="scroll"><table><thead><tr><th>名称</th><th>题量</th></tr></thead><tbody>${list || `<tr><td colspan="2" class="muted">还没有题库</td></tr>`}</tbody></table></div>
    </section>
    <section class="panel">
      ${
        bankId
          ? `<div class="head-row">
               <h2>${esc(bankName)}</h2>
               <div class="acts">
                 <button type="button" data-click="export:${esc(bankId)}">导出</button>
                 <button type="button" class="danger" data-click="del-bank:${esc(bankId)}">删除</button>
               </div>
             </div>
              <details class="fold">
                <summary>从 HTML 导入</summary>
                <p class="hint">使用规范化题库 HTML（按题型分节、每题含题干/选项/答案）。</p>
                <form name="bank-import" class="row">
                  <label class="field">选择文件<input type="file" name="file" accept=".html,text/html" required /></label>
                  <button type="submit">导入</button>
                </form>
              </details>
              <section class="subpanel">
                <div class="head-row">
                  <h3>题目</h3>
                 <button type="button" data-add-q>单题增加</button>
                </div>
                ${showQForm ? questionFormHtml() : ""}
                <div class="scroll" data-fit="bankq">${questionSheetHtml(qs, qSlice.from, qSlice.count, true)}</div>
                ${pagerHtml("bankq", bankQPage, qSlice.pages, qs.length)}
              </section>`
          : "<h2>选择左侧题库</h2><p class=\"hint\">点左侧题库名称开始维护。</p>"
      }
    </section>
  </div>`;
}

function rosterHtml(): string {
  const filtered = students.filter((s) => !classFilter || String(s.class_name) === classFilter);
  const stuSize = pageSizeFor("stu");
  const stuPages = Math.max(1, Math.ceil((filtered.length || 1) / stuSize));
  if (rosterPage > stuPages) {
    rosterPage = stuPages;
  }
  const stuFrom = filtered.length ? (rosterPage - 1) * stuSize : 0;
  const pageItems = filtered.slice(stuFrom, stuFrom + stuSize);
  const pageIds = pageItems.map((s) => String(s.id));
  const pageAll = pageIds.length > 0 && pageIds.every((id) => selectedStu.has(id));
  const picked = selectedStu.size;
  const selectedClass = classes.find((c) => c.name === classFilter);
  const rows = pageItems
    .map(
      (s) =>
        `<tr class="pick${selectedStu.has(String(s.id)) ? " on" : ""}" data-stu-row="${esc(s.id)}">
          <td class="tick"><input type="checkbox"${selectedStu.has(String(s.id)) ? " checked" : ""} /></td>
          <td>${esc(s.class_name)}</td><td class="mono">${esc(s.student_no)}</td><td>${esc(s.name)}</td>
        </tr>`,
    )
    .join("");
  const classRows = classes
    .map(
      (c) =>
        `<tr class="pick${classFilter === c.name ? " on" : ""}" data-class="${esc(c.name)}">
          <td>${esc(c.name)}</td>
          <td>${esc(c.student_count)}</td>
        </tr>`,
    )
    .join("");
  return `<section class="panel">
    <div class="split">
      <div class="col">
        <div class="head-row">
          <h2>班级</h2>
          ${selectedClass ? `<button type="button" class="danger" data-click="del-class:${esc(selectedClass.id)}">删除班级</button>` : ""}
        </div>
        <form name="class-create" class="row">
          <label class="field">班级名称<input name="name" required placeholder="例如：高一1班" /></label>
          <button class="primary" type="submit">新建班级</button>
        </form>
        <div class="scroll"><table><thead><tr><th>班级</th><th>人数</th></tr></thead><tbody>${classRows || `<tr><td colspan="2" class="muted">还没有班级</td></tr>`}</tbody></table></div>
      </div>
      <div class="col">
        <h2>学生</h2>
        <p class="toolbar">
          <button type="button" data-click="tpl:x">下载模板</button>
          <button type="button" data-click="stu-csv:x">导出 CSV</button>
          ${classFilter ? `<button type="button" data-class="">显示全部</button>` : ""}
        </p>
        <form name="student-create" class="row">
          <label class="field">班级<input name="class_name" required value="${esc(classFilter)}" list="class-list" /></label>
          <label class="field">学号<input name="student_no" required /></label>
          <label class="field">姓名<input name="name" required /></label>
          <button class="primary" type="submit">登记</button>
        </form>
        <datalist id="class-list">${classes.map((c) => `<option value="${esc(c.name)}"></option>`).join("")}</datalist>
        <details class="fold">
          <summary>导入 CSV</summary>
          <form name="csv">
            <div class="row">
              <label class="field">CSV 文件<input type="file" name="file" accept=".csv,.txt,.tsv,text/csv,text/plain" /></label>
            </div>
            <label class="field">或直接粘贴<textarea name="csv" rows="4" placeholder="班级,学号,姓名"></textarea></label>
            <p class="hint">Excel 请另存为 CSV；也可从表格复制三列直接粘贴。表头为班级、学号、姓名。</p>
            <p class="toolbar"><button class="primary" type="submit">导入</button></p>
          </form>
        </details>
        <div class="head-row">
          <p class="hint" data-stu-hint>${classFilter ? `当前班级 ${esc(classFilter)} · ` : ""}共 ${esc(filtered.length)} 人${picked ? ` · 已选 ${picked}` : ""}</p>
          <div class="acts">
            <button type="button" class="danger" data-del-stus${picked ? "" : " hidden"}>删除所选（${picked}）</button>
          </div>
        </div>
        <div class="scroll" data-fit="stu"><table><thead><tr><th class="tick"><input type="checkbox" data-pick-page${pageAll ? " checked" : ""} /></th><th>班级</th><th>学号</th><th>姓名</th></tr></thead><tbody>${rows || `<tr><td colspan="4" class="muted">名册为空</td></tr>`}</tbody></table></div>
        ${pagerHtml("stu", rosterPage, stuPages, filtered.length)}
      </div>
    </div>
  </section>`;
}

function splitSubtotal(n: number, sub: number): number[] {
  const count = Math.max(1, n);
  const total = Math.max(count, Math.max(0, sub));
  const q = Math.floor(total / count);
  const r = total % count;
  return Array.from({ length: count }, (_, i) => q + (i >= count - r ? 1 : 0));
}

function planSum(plan: Record<string, number[]>): number {
  return Object.values(plan).reduce((s, arr) => s + arr.reduce((a, b) => a + b, 0), 0);
}

function scoreCaptionHtml(qtype: string, arr: number[]): string {
  if (!arr.length) {
    return "";
  }
  const z = arr.reduce((a, b) => a + b, 0);
  const runs: Array<{ start: number; end: number; val: number }> = [];
  arr.forEach((v, i) => {
    const n = i + 1;
    const last = runs[runs.length - 1];
    if (last && last.val === v) {
      last.end = n;
    } else {
      runs.push({ start: n, end: n, val: v });
    }
  });
  const sub = `<input type="text" inputmode="numeric" class="score-inline" data-score-sub value="${esc(z)}">`;
  const runInp = (g: { start: number; end: number; val: number }, i: number) =>
    `<input type="text" inputmode="numeric" class="score-inline" data-score-run="${i}" data-score-from="${g.start}" data-score-to="${g.end}" value="${esc(g.val)}">`;
  let inner = "";
  if (runs.length === 1) {
    inner = `每题${runInp(runs[0], 0)}分，共${sub}分`;
  } else {
    const parts = runs.map((g, i) => {
      if (g.start === g.end) {
        return `第${g.start}题${runInp(g, i)}分`;
      }
      return `${g.start}到${g.end}题每题${runInp(g, i)}分`;
    });
    inner = `${parts.join("，")}，共${sub}分`;
  }
  return ` <span class="score-cap" data-score-type="${esc(qtype)}">（${inner}）</span>`;
}

function currentPaperPlan(): Record<string, number[]> {
  const paper = papers.find((p) => p.id === paperId);
  const raw = (paper?.score_plan as Record<string, number[]>) || {};
  const out: Record<string, number[]> = {};
  for (const [k, arr] of Object.entries(raw)) {
    out[k] = Array.isArray(arr) ? [...arr] : [];
  }
  return out;
}

async function savePaperScores(inp: HTMLInputElement): Promise<void> {
  if (!paperId) {
    return;
  }
  const cap = inp.closest<HTMLElement>("[data-score-type]");
  if (!cap) {
    return;
  }
  const qtype = cap.dataset.scoreType || "";
  const plan = currentPaperPlan();
  const arr = [...(plan[qtype] || [])];
  if (!arr.length) {
    return;
  }
  if (inp.matches("[data-score-sub]")) {
    plan[qtype] = splitSubtotal(arr.length, Math.max(arr.length, Number.parseInt(inp.value, 10) || arr.length));
  } else {
    const from = Number(inp.dataset.scoreFrom || 1);
    const to = Number(inp.dataset.scoreTo || from);
    const val = Math.max(1, Number.parseInt(inp.value, 10) || 1);
    for (let i = from - 1; i < to && i < arr.length; i++) {
      arr[i] = val;
    }
    plan[qtype] = arr;
  }
  const total = planSum(plan);
  const r = await tcall(`/api/teacher/papers/${paperId}`, {
    method: "PATCH",
    body: JSON.stringify({ score_plan: plan, total_score: total }),
  });
  const paper = papers.find((p) => p.id === paperId);
  if (r.status === 204 && paper) {
    paper.score_plan = plan;
    paper.total_score = total;
    if (Number(paper.pass_score) > total) {
      paper.pass_score = Math.floor(total * 0.6);
    }
    if (!scoreEditing()) {
      render();
    }
  } else {
    tell(errMsg(r.data), "err");
  }
}

function bindPaperScores(): void {
  const sheet = root.querySelector(".qsheet");
  if (!sheet || tab !== "papers") {
    return;
  }
  const scoreInp = (el: EventTarget | null): HTMLInputElement | null =>
    el instanceof HTMLInputElement && el.matches(".score-inline") ? el : null;
  sheet.addEventListener("mousedown", (ev) => {
    if (scoreInp(ev.target)) {
      ev.stopPropagation();
    }
  });
  sheet.addEventListener("click", (ev) => {
    if (scoreInp(ev.target)) {
      ev.stopPropagation();
    }
  });
  sheet.addEventListener("keydown", (ev) => {
    if ((ev as KeyboardEvent).key !== "Enter" || !scoreInp(ev.target)) {
      return;
    }
    ev.preventDefault();
    (ev.target as HTMLInputElement).blur();
  });
  sheet.addEventListener("input", (ev) => {
    const t = scoreInp(ev.target);
    if (!t) {
      return;
    }
    const cleaned = t.value.replace(/\D+/g, "");
    if (t.value !== cleaned) {
      t.value = cleaned;
    }
    if (!t.matches("[data-score-run]")) {
      return;
    }
    const cap = t.closest<HTMLElement>("[data-score-type]");
    const sub = cap?.querySelector<HTMLInputElement>("[data-score-sub]");
    if (!cap || !sub) {
      return;
    }
    let sum = 0;
    cap.querySelectorAll<HTMLInputElement>("[data-score-run]").forEach((inp) => {
      const from = Number(inp.dataset.scoreFrom || 1);
      const to = Number(inp.dataset.scoreTo || from);
      sum += Math.max(1, Number.parseInt(inp.value, 10) || 1) * (to - from + 1);
    });
    sub.value = String(sum);
  });
  sheet.addEventListener("focusout", (ev) => {
    const t = scoreInp(ev.target);
    if (!t) {
      return;
    }
    const next = (ev as FocusEvent).relatedTarget as HTMLElement | null;
    if (next && next.closest(".score-cap") === t.closest(".score-cap")) {
      return;
    }
    void savePaperScores(t);
  });
}

function papersHtml(): string {
  const list = papers
    .map((p) => {
      const counts = p.type_counts as Record<string, number> | undefined;
      const c = counts
        ? Object.entries(counts)
            .map(([k, n]) => `${qlabel(k)}${n}`)
            .join(" / ")
        : "";
      return `<tr class="pick${paperId === p.id ? " on" : ""}" data-paper="${esc(p.id)}">
        <td>${esc(p.title)}</td>
        <td>${esc(paperKind(p))}</td>
        <td>${esc(p.total_score)}</td>
        <td>${esc(c)}</td>
      </tr>`;
    })
    .join("");
  const bankRows = banks
    .map(
      (b) => `<div class="bank-draw">
        <label class="check">
          <input type="checkbox" name="bank_id" value="${esc(b.id)}" />
          <span>${esc(b.name)}（${esc(b.question_count)}题）</span>
        </label>
        <label class="field slim">抽题数<input type="number" min="0" name="draw_count_${esc(b.id)}" placeholder="数量" /></label>
        <label class="field slim">比例%<input type="number" min="0" max="100" step="0.1" name="draw_ratio_${esc(b.id)}" placeholder="%" /></label>
      </div>`,
    )
    .join("");
  const counts = QTYPES.map(
    ([k, l]) => `<label class="field">${l}题量<input type="number" min="0" name="count_${k}" value="0" /></label>`,
  ).join("");
  const qSlice = questionSlice(paperNeedPack, paperPackStarts, paperQPage, paperQs.length);
  if (paperQPage > qSlice.pages) {
    paperQPage = qSlice.pages;
  }
  const current = papers.find((p) => p.id === paperId);
  const dialog = showPaperForm
    ? `<div class="modal" data-modal>
        <div class="dialog">
          <div class="head-row">
            <h2>新建试卷</h2>
            <button type="button" class="ghost" data-close-modal>关闭</button>
          </div>
          <form name="paper-create">
            <label class="field">试卷标题<input name="title" required placeholder="例如：第3周课堂测验卷" /></label>
            <div class="row">
              <label class="field">总分<input name="total_score" type="number" value="100" /></label>
              <label class="field">及格分<input name="pass_score" type="number" placeholder="默认 60%" /></label>
            </div>
            <div class="checks" style="max-height:none;background:transparent;border:0;padding:0">
              <label class="check"><input type="checkbox" name="shuffle_within_section" checked /> <span>题型内乱序</span></label>
              <label class="check"><input type="checkbox" name="shuffle_options" checked /> <span>选项乱序</span></label>
              <label class="check"><input type="checkbox" name="live_draw" /> <span>智能实时组卷</span></label>
              <label class="check"><input type="checkbox" name="auto_grade" checked /> <span>自动批改填空题</span></label>
            </div>
            <p class="hint">勾选「智能实时组卷」后每次预览和每位考生都会重新抽题；不勾选则创建时生成一份固定试卷。单选/多选/判断有参考答案即自动批改。关闭「自动批改填空题」后填空走给分台；编程题与空参考答案始终手工。</p>
            <div class="row">${counts}</div>
            <p class="toolbar"><button type="button" data-toggle="bank_id">全选题库</button></p>
            <p class="hint">每个题库可填抽题数量或比例；都留空则只按题型题量混合抽取。若填写，勾选的题库都要填，且数量之和等于各题型题量之和。</p>
            <div class="bank-draws">${bankRows || `<p class="muted">请先在题库管理中建库</p>`}</div>
            <button class="primary" type="submit">保存试卷</button>
          </form>
        </div>
      </div>`
    : "";
  const right = current
    ? `<div class="head-row">
         <h2>${esc(current.title)}</h2>
         <div class="acts">
           <button type="button" class="danger" data-click="del-paper:${esc(paperId)}">删除试卷</button>
         </div>
       </div>
       <p class="hint">总分 ${esc(current.total_score)} · 及格 ${esc(current.pass_score)} · ${esc(
         paperKind(current)
       )} · ${esc(
         ((current.type_counts as Record<string, number> | undefined)
           ? Object.entries(current.type_counts as Record<string, number>)
               .map(([k, n]) => `${qlabel(k)}${n}`)
               .join(" / ")
           : "") || "未填题量"
       )}</p>
       ${paperPreviewErr ? `<p class="hint">${esc(paperPreviewErr)}</p>` : ""}
          <div class="scroll" data-fit="paperq">${questionSheetHtml(paperQs, qSlice.from, qSlice.count, false, (current?.score_plan as Record<string, number[]>) || undefined)}</div>
          ${pagerHtml("paperq", paperQPage, qSlice.pages, paperQs.length)}`
    : `<h2>试卷题目</h2><p class="hint">点左侧试卷查看题目，或点「新建试卷」组卷。</p>`;
  return `<div class="grid papers">
    <section class="panel">
      <div class="head-row">
        <h2>已组试卷</h2>
        <button class="primary" type="button" data-paper-new>新建试卷</button>
      </div>
      <div class="scroll"><table><thead><tr><th>标题</th><th>类型</th><th>总分</th><th>题量</th></tr></thead>
      <tbody>${list || `<tr><td colspan="4" class="muted">还没有试卷</td></tr>`}</tbody></table></div>
    </section>
    <section class="panel">${right}</section>
  </div>${dialog}`;
}

function taskCreateHtml(): string {
  const stuItems = students.map((s) => ({
    id: String(s.id),
    label: `${s.class_name} ${s.student_no} ${s.name}`,
    extra: `${s.class_name} ${s.student_no} ${s.name}`,
  }));
  const paperOpts = papers
    .map((p) => `<option value="${esc(p.id)}">${esc(p.title)}（${esc(p.total_score)}分 · ${esc(paperKind(p))}）</option>`)
    .join("");
  const classItems = classes.map((c) => ({
    id: String(c.name),
    label: `${c.name}（${c.student_count}人）`,
  }));
  return `<form name="task-create">
      <label class="field">标题<input name="title" required placeholder="例如：第3周课堂测验" /></label>
      <div class="row">
        <label class="field">模式<select name="mode"><option value="exam">考试</option><option value="practice">练习</option></select></label>
        <label class="field">时长（分钟）<input name="duration_minutes" type="number" value="45" /></label>
        <label class="check early-flag"><input type="checkbox" name="allow_early_submit" /> <span>允许提前交卷</span></label>
        <label class="check early-flag"><input type="checkbox" name="auto_grade" checked /> <span>自动批改填空题</span></label>
      </div>
      <label class="field">选用试卷<select name="paper_id" required><option value="">请选择已组试卷</option>${paperOpts}</select></label>
      <h3>参加班级</h3>
      ${checkList("class_name", classItems, "全选班级")}
      <details class="fold">
        <summary>或点名到人</summary>
        <div class="picker-head"><input id="stu-filter" placeholder="筛选班级 / 学号 / 姓名" /></div>
        ${checkList("student_id", stuItems, "全选可见")}
      </details>
      <button class="primary" type="submit">生成任务</button>
    </form>`;
}

function studentUrlBox(): string {
  const urls = studentUrls();
  if (!urls.length) {
    return "<p class=\"hint\">暂无局域网地址</p>";
  }
  return urls
    .map(
      (url) => `<div class="url-box lan">
        <a href="${esc(url)}" target="_blank">${esc(url)}</a>
        <button type="button" class="ghost" data-copy="${esc(url)}">复制</button>
      </div>`,
    )
    .join("");
}

function taskDetailHtml(): string {
  if (!task) {
    return `<p class="hint">选择左侧任务查看详情。</p>`;
  }
  const status = String(task.status);
  const draft = status === "draft";
  const published = status === "published";
  const historic = isHistory(status);
  const isChild = Boolean(task.parent_id);
  const classHint = childClassName();
  const ids = ((roster?.student_ids as string[]) || []);
  const pool = classHint
    ? students.filter((s) => String(s.class_name) === classHint)
    : students;
  const stuItems = pool.map((s) => ({
    id: String(s.id),
    label: `${s.class_name} ${s.student_no} ${s.name}`,
    extra: `${s.class_name} ${s.student_no} ${s.name}`,
    checked: ids.includes(String(s.id)),
  }));
  const classCount = uniqueClasses(
    ids.length ? students.filter((s) => ids.includes(String(s.id))) : students,
  ).length;
  const early = task.allow_early_submit ? "checked" : "";
  const meta = published || historic
    ? `关闭于 ${esc(task.closes_at || "—")}`
    : `${esc(task.duration_minutes)} 分钟`;
  const earlyBox = `<label class="check early-flag"><input type="checkbox" data-early ${early} /> <span>允许提前交卷</span></label>`;
  const autoOn = task.auto_grade !== false ? "checked" : "";
  const autoLock = status === "released" ? " disabled" : "";
  const autoBox = `<label class="check early-flag"><input type="checkbox" data-auto-grade ${autoOn}${autoLock} /> <span>自动批改填空题</span></label>`;
  let body = `<p>${esc(modeLabel(task.mode))} · <span class="badge ${badgeFor(status)}">${esc(statusLabel(status))}</span> · ${meta}</p>`;
  if (draft) {
    const splitBtn =
      !isChild && !classHint && classCount > 1
        ? `<button type="button" data-click="split:${esc(taskId)}">按班拆分子任务</button>`
        : "";
    return `${body}<p class="toolbar">
        <button class="primary" type="button" data-click="publish:${esc(taskId)}">发布</button>
        ${splitBtn}
        <button class="danger" type="button" data-click="del-task:${esc(taskId)}">删除任务</button>
        ${earlyBox}
        ${autoBox}
      </p>
      <h3>答卷入口</h3>
      ${studentUrlBox()}
      <form name="roster" class="roster-form">
        <div class="head-row">
          <h3>任务名单${classHint ? ` · ${esc(classHint)}` : ""}</h3>
          <button class="primary" type="submit">保存名单</button>
        </div>
        <div class="picker-head"><input id="stu-filter" placeholder="筛选班级 / 学号 / 姓名" /></div>
        ${checkList("student_id", stuItems, "全选可见")}
      </form>`;
  }
  if (published) {
    return `${body}<p class="toolbar">
        <button class="danger" type="button" data-click="close:${esc(taskId)}">收卷</button>
        <button class="danger" type="button" data-click="end:${esc(taskId)}">结束任务</button>
        ${earlyBox}
        ${autoBox}
      </p>
      <h3>答卷入口</h3>
      ${studentUrlBox()}
      <h3>监控</h3>
      <div id="monitor-mount">${monitorInner()}</div>`;
  }
  return `${body}
    <p class="toolbar">${autoBox}</p>
    <h3>答卷入口</h3>
    ${studentUrlBox()}
    <h3>当时监控</h3>
    <div id="monitor-mount">${monitorInner()}</div>`;
}

function fmtScore(v: unknown, d = 1): string {
  const n = Number(v);
  if (!Number.isFinite(n)) {
    return "—";
  }
  if (d === 0) {
    return String(Math.round(n));
  }
  return Number.isInteger(n) ? String(n) : n.toFixed(d);
}

function fmtPct(v: unknown): string {
  const n = Number(v);
  if (!Number.isFinite(n)) {
    return "—";
  }
  return `${(n * 100).toFixed(1)}%`;
}

function paperKind(p: Record<string, unknown>): string {
  return p.live_draw ? "智能实时组卷" : "固定试卷";
}

function fmtPctNum(v: unknown): string {
  const n = Number(v);
  if (!Number.isFinite(n)) {
    return "—";
  }
  return `${n.toFixed(1)}%`;
}

function showAns(v: unknown): string {
  if (v == null || v === "") {
    return "未作答";
  }
  if (typeof v === "object" && v && "value" in (v as object)) {
    return showAns((v as { value?: unknown }).value);
  }
  if (Array.isArray(v)) {
    return v.length ? v.map(String).join("、") : "未作答";
  }
  if (typeof v === "boolean") {
    return v ? "对" : "错";
  }
  return String(v);
}

function choiceKeys(v: unknown): string[] {
  if (v == null || v === "") {
    return [];
  }
  if (typeof v === "object" && v && "value" in (v as object)) {
    return choiceKeys((v as { value?: unknown }).value);
  }
  if (Array.isArray(v)) {
    return v.map(String);
  }
  if (typeof v === "boolean") {
    return [];
  }
  return [String(v)];
}

function reviewOptionsHtml(item: Record<string, unknown>): string {
  const t = String(item.qtype || "");
  if (t !== "single" && t !== "multiple") {
    return "";
  }
  const opts = (item.options as Array<{ key?: string; text?: string }>) || [];
  if (!opts.length) {
    return "";
  }
  const mine = new Set(choiceKeys(item.response));
  const right = new Set(choiceKeys(item.answer));
  return opts
    .map((o) => {
      const picked = mine.has(String(o.key || ""));
      const ok = right.has(String(o.key || ""));
      const cls = ["opt", "ro", picked ? "mine" : "", ok ? "right" : ""].filter(Boolean).join(" ");
      const tag = ok ? "正确答案" : picked ? "学生选择" : "";
      return `<div class="${cls}"><span><b>${esc(o.key)}.</b> ${esc(o.text)}</span>${tag ? `<em>${esc(tag)}</em>` : ""}</div>`;
    })
    .join("");
}

function sectionAgg(items: Array<Record<string, unknown>>): { cr: number; sr: number; avg: number; pts: number } {
  let nSum = 0;
  let correct = 0;
  let avg = 0;
  let pts = 0;
  for (const q of items) {
    const n = Number(q.n) || 0;
    nSum += n;
    correct += (Number(q.correct_rate) || 0) * n;
    avg += Number(q.avg_score) || 0;
    pts += Number(q.points) || 0;
  }
  return {
    cr: nSum ? correct / nSum : 0,
    sr: pts ? avg / pts : 0,
    avg,
    pts,
  };
}

function rateBar(v: unknown): number {
  const n = Number(v);
  if (!Number.isFinite(n)) {
    return 0;
  }
  return Math.max(0, Math.min(100, n <= 1 ? n * 100 : n));
}

function gradeStat(label: string, value: string, pane?: string, sub?: string): string {
  const click = pane ? ` data-grade-pane="${esc(pane)}"` : "";
  return `<button type="button" class="stat"${click}><b>${esc(value)}</b><span>${esc(label)}</span>${sub ? `<em>${esc(sub)}</em>` : ""}</button>`;
}

function tasksHtml(): string {
  const history = taskView === "history";
  const filtered = tasks.filter((t) => (history ? isHistory(t.status) : !isHistory(t.status)));
  const list = filtered
    .map(
      (t) =>
        `<tr class="pick${taskId === t.id ? " on" : ""}" data-task="${esc(t.id)}"><td class="clip">${clipTitle(t.title)}</td>
         <td class="mode">${esc(modeLabel(t.mode))}</td>
         <td class="st"><span class="badge ${badgeFor(t.status)}">${esc(statusLabel(t.status))}</span></td>
         <td class="num">${esc(t.total_score)}</td></tr>`,
    )
    .join("");
  const dialog = showTaskForm
    ? `<div class="modal" data-modal>
        <div class="dialog">
          <div class="head-row">
            <h2>新建任务</h2>
            <button type="button" class="ghost" data-close-modal>关闭</button>
          </div>
          ${taskCreateHtml()}
        </div>
      </div>`
    : "";
  const seg = `<div class="seg">
    <button type="button" data-task-view="current" class="${taskView === "current" ? "on" : ""}">当前任务</button>
    <button type="button" data-task-view="history" class="${taskView === "history" ? "on" : ""}">历史任务</button>
  </div>`;
  const emptyHint = history ? "还没有已结束的历史任务。" : "还没有草稿或进行中的任务。";
  return `<div class="task-wrap">${seg}<div class="grid tasks">
    <section class="panel">
      <div class="head-row">
        <h2>${history ? "历史任务" : "当前任务"}</h2>
        <button class="primary" type="button" data-task-new>新建任务</button>
      </div>
      <div class="scroll"><table class="task-list"><thead><tr><th class="clip">标题</th><th class="mode">模式</th><th class="st">状态</th><th class="num">总分</th></tr></thead>
      <tbody>${list || `<tr><td colspan="4" class="muted">${emptyHint}</td></tr>`}</tbody></table></div>
    </section>
    <section class="panel">
      <h2>${task ? clipTitle(task.title) : "任务详情"}</h2>
      ${taskDetailHtml()}
    </section>
  </div>${dialog}</div>`;
}

function vbarChartHtml(title: string, items: Array<[string, number]>): string {
  const max = Math.max(1, ...items.map(([, n]) => n));
  return `<div class="chart">
    <h3>${esc(title)}</h3>
    <div class="vbars">${items
      .map(([lab, n]) => {
        const h = Math.max(4, Math.round((n / max) * 120));
        return `<div class="vbar"><span>${esc(n)}</span><i style="height:${h}px"></i><em>${esc(lab)}</em></div>`;
      })
      .join("")}</div>
  </div>`;
}

function hbarChartHtml(title: string, items: Array<[string, number, string]>, pane?: string): string {
  if (!items.length) {
    return "";
  }
  const click = pane ? ` data-grade-pane="${esc(pane)}"` : "";
  return `<div class="chart"${click}>
    <h3>${esc(title)}</h3>
    <div class="hbars">${items
      .map(
        ([lab, rate, extra]) =>
          `<div class="hbar"><span>${esc(lab)}</span><div class="track"><i class="fill" style="width:${rateBar(rate)}%"></i></div><em>${esc(extra)}</em></div>`,
      )
      .join("")}</div>
  </div>`;
}

function donutChartHtml(title: string, parts: Array<[string, number, string]>): string {
  const total = parts.reduce((s, [, n]) => s + Math.max(0, n), 0);
  if (!total) {
    return "";
  }
  let acc = 0;
  const stops = parts
    .map(([, n, color]) => {
      const from = acc;
      acc += (Math.max(0, n) / total) * 100;
      return `${color} ${from.toFixed(2)}% ${acc.toFixed(2)}%`;
    })
    .join(", ");
  const legend = parts
    .map(([lab, n, color]) => {
      const pct = total ? ((Math.max(0, n) / total) * 100).toFixed(1) : "0.0";
      return `<li><i style="background:${esc(color)}"></i>${esc(lab)} ${esc(n)} · ${esc(pct)}%</li>`;
    })
    .join("");
  return `<div class="chart"><h3>${esc(title)}</h3><div class="donut-row"><div class="donut" style="background:conic-gradient(${stops})"></div><ul class="legend">${legend}</ul></div></div>`;
}

function rankTableHtml(title: string, rows: Array<Record<string, unknown>>): string {
  const body = rows
    .map(
      (r) =>
        `<tr><td class="mono">${esc(r.student_no)}</td><td>${esc(r.name)}</td><td>${esc(r.class_name)}</td><td>${esc(r.total_score)}</td></tr>`,
    )
    .join("");
  return `<div class="chart"><h3>${esc(title)}</h3>
    <table class="rank"><thead><tr><th>学号</th><th>姓名</th><th>班级</th><th>总分</th></tr></thead>
    <tbody>${body || `<tr><td colspan="4" class="muted">暂无</td></tr>`}</tbody></table></div>`;
}

function codeResponseText(c: Record<string, unknown>): string {
  const resp = c.response;
  if (typeof resp === "object" && resp && "value" in (resp as object)) {
    return String((resp as { value?: unknown }).value ?? "");
  }
  if (resp == null) {
    return "";
  }
  return typeof resp === "string" ? resp : JSON.stringify(resp);
}

function markingCanScore(status = String(task?.status || "")): boolean {
  return status === "published" || status === "closed" || status === "graded";
}

function codePending(r: Record<string, unknown>): boolean {
  return ((r.code_items as Array<Record<string, unknown>>) || []).some((c) => c.score == null || c.score === "");
}

function markingPending(r: Record<string, unknown>): boolean {
  const codes = (r.code_items as Array<Record<string, unknown>>) || [];
  if (!codes.length) {
    return false;
  }
  if (codePending(r)) {
    return true;
  }
  return !r.marking_confirmed;
}

function clampScore(n: number, max: number): number {
  if (!Number.isFinite(n)) {
    return 0;
  }
  return Math.max(0, Math.min(max, Math.round(n)));
}

const GRADE_STOPS: [number, number, number][] = [
  [255, 212, 0],
  [252, 175, 23],
  [237, 25, 65],
];

function mixRgb(t: number, stops: [number, number, number][]): [number, number, number] {
  const u = Math.min(1, Math.max(0, t));
  const segs = stops.length - 1;
  const x = u * segs;
  const i = Math.min(segs - 1, Math.floor(x));
  const f = x - i;
  const a = stops[i];
  const b = stops[i + 1];
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const srgb = (v: number) => {
    const y = v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(1, Math.max(0, y)) * 255);
  };
  return [
    srgb(lin(a[0]) * (1 - f) + lin(b[0]) * f),
    srgb(lin(a[1]) * (1 - f) + lin(b[1]) * f),
    srgb(lin(a[2]) * (1 - f) + lin(b[2]) * f),
  ];
}

function rgbCss(c: [number, number, number]): string {
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

function scorePadStyle(n: number, coloredIndex: number, coloredCount: number): string {
  if (n <= 0 || coloredCount <= 0) {
    return "background:#c5ccd4;color:#102018";
  }
  const t0 = coloredCount <= 1 ? 0 : coloredIndex / coloredCount;
  const t1 = coloredCount <= 1 ? 1 : (coloredIndex + 1) / coloredCount;
  const a = mixRgb(t0, GRADE_STOPS);
  const b = mixRgb(t1, GRADE_STOPS);
  const m = mixRgb((t0 + t1) / 2, GRADE_STOPS);
  const ink = (0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2]) / 255 > 0.58 ? "#102018" : "#fff";
  return `background:linear-gradient(to right, ${rgbCss(a)}, ${rgbCss(b)});color:${ink}`;
}

function scorePadStep(max: number): number {
  const cap = 13;
  if (max <= cap - 1) {
    return 1;
  }
  const raw = Math.ceil(max / (cap - 1));
  const nice = [1, 2, 5, 10, 20, 25, 50, 100];
  return nice.find((n) => n >= raw) || raw;
}

function scorePadValues(max: number): number[] {
  if (max <= 0) {
    return [0];
  }
  const step = scorePadStep(max);
  const vals: number[] = [];
  for (let n = 0; n <= max; n += step) {
    vals.push(n);
  }
  if (vals[vals.length - 1] !== max) {
    vals.push(max);
  }
  return vals;
}

function markingRows(): Array<Record<string, unknown>> {
  return (results?.results as Array<Record<string, unknown>>) || [];
}

function ensureMarkingStudent(): void {
  const rows = markingRows();
  if (!rows.length) {
    gradeStu = "";
    return;
  }
  if (gradeStu && rows.some((r) => String(r.attempt_id) === gradeStu)) {
    return;
  }
  const pending = rows.find(markingPending);
  gradeStu = String((pending || rows[0]).attempt_id || "");
}

function nextPendingAttempt(current: string): string {
  const rows = markingRows();
  const idx = rows.findIndex((r) => String(r.attempt_id) === current);
  const ordered = idx >= 0 ? rows.slice(idx + 1).concat(rows.slice(0, idx)) : rows;
  const next = ordered.find(markingPending);
  return next ? String(next.attempt_id) : "";
}

async function saveOneCodeScore(attempt: string, qid: string, score: number, max: number): Promise<void> {
  if (!attempt || !qid || !markingCanScore()) {
    return;
  }
  const s = clampScore(score, max);
  const r = await tcall(`/api/teacher/attempts/${attempt}/code-scores`, {
    method: "PUT",
    body: JSON.stringify({ scores: { [qid]: s } }),
  });
  if (r.status >= 400) {
    tell(errMsg(r.data), "err");
    return;
  }
  const row = markingRows().find((x) => String(x.attempt_id) === attempt);
  if (row) {
    const items = (row.code_items as Array<Record<string, unknown>>) || [];
    const it = items.find((c) => String(c.question_id) === qid);
    if (it) {
      it.score = s;
    }
    row.marking_confirmed = false;
    const data = r.data as { total_score?: unknown };
    if (data?.total_score != null) {
      row.total_score = data.total_score;
    }
  }
  render();
}

function scorePadsHtml(attempt: string, qid: unknown, points: unknown, score: unknown, locked: boolean): string {
  const max = Math.max(0, Number(points) || 0);
  const cur = score == null || score === "" ? null : Number(score);
  const vals = scorePadValues(max);
  const coloredCount = vals.reduce((n, v) => n + (v > 0 ? 1 : 0), 0);
  let coloredIndex = -1;
  const pads = vals
    .map((n) => {
      if (n > 0) {
        coloredIndex += 1;
      }
      const on = cur != null && Number.isFinite(cur) && cur === n;
      return `<button type="button" class="score-pad${on ? " on" : ""}" data-score-n="${n}" data-qid="${esc(qid)}" data-attempt="${esc(attempt)}" data-max="${max}" style="${scorePadStyle(n, coloredIndex, coloredCount)}"${locked ? " disabled" : ""}>${n}</button>`;
    })
    .join("");
  return `<div class="score-row">
    <div class="score-pads">${pads}</div>
    <span class="score-num"><input data-score-input data-qid="${esc(qid)}" data-attempt="${esc(attempt)}" type="number" min="0" max="${max}" step="1" value="${cur ?? ""}"${locked ? " disabled" : ""} /><span>分</span></span>
  </div>`;
}

function markingHtml(): string {
  const markTasks = tasks.filter((t) => String(t.status) !== "draft");
  const list = markTasks
    .map(
      (t) =>
        `<tr class="pick${taskId === t.id ? " on" : ""}" data-task="${esc(t.id)}"><td class="clip">${clipTitle(t.title)}</td>
         <td class="st"><span class="badge ${badgeFor(t.status)}">${esc(statusLabel(t.status))}</span></td>
         <td class="num">${esc(t.total_score)}</td></tr>`,
    )
    .join("");
  const ready = Boolean(task && String(task.status) !== "draft");
  if (ready) {
    ensureMarkingStudent();
  }
  return `<div class="grid marking">
    <section class="panel">
      <h2>考试任务</h2>
      <div class="scroll"><table class="task-list"><thead><tr><th class="clip">标题</th><th class="st">状态</th><th class="num">总分</th></tr></thead>
      <tbody>${list || `<tr><td colspan="3" class="muted">还没有可批改的任务</td></tr>`}</tbody></table></div>
    </section>
    <section class="panel">${ready ? markingStudentsHtml() : `<h2>考生</h2><p class="hint">先选左侧任务。</p>`}</section>
    <section class="panel">${ready ? markingDeskHtml() : `<h2>给分台</h2><p class="hint">点色块即给分。给完该生待批改题后点提交确认。全部确认且任务结束后可公开试卷。</p>`}</section>
  </div>${gradePaperDialogHtml()}`;
}

function markingStudentsHtml(): string {
  const rows = markingRows();
  const scoreN = rows.filter(codePending).length;
  const confirmN = rows.filter((r) => markingPending(r) && !codePending(r)).length;
  const body = rows
    .map((r) => {
      const codes = (r.code_items as Array<Record<string, unknown>>) || [];
      const picked = gradeStu === String(r.attempt_id);
      let mark = "—";
      if (codes.length) {
        if (codePending(r)) {
          mark = `<span class="badge warn">待给分</span>`;
        } else if (!r.marking_confirmed) {
          mark = `<span class="badge warn">待确认</span>`;
        } else {
          mark = `<span class="badge ok">已确认</span>`;
        }
      }
      return `<tr class="pick${picked ? " on" : ""}" data-grade-stu="${esc(r.attempt_id)}">
        <td class="mono">${esc(r.student_no)}</td><td>${esc(r.name)}</td>
        <td>${esc(r.total_score)}</td><td>${mark}</td>
      </tr>`;
    })
    .join("");
  const tally = scoreN || confirmN ? `待给分 ${scoreN} · 待确认 ${confirmN}` : "全部已确认";
  return `<div class="head-row">
      <h2>考生</h2>
      <p class="hint">${tally} · 共 ${rows.length}</p>
    </div>
    <div class="scroll"><table class="stu-compact">
      <thead><tr><th>学号</th><th>姓名</th><th>总分</th><th>批改</th></tr></thead>
      <tbody>${body || `<tr><td colspan="4" class="muted">还没有交卷记录</td></tr>`}</tbody>
    </table></div>`;
}

function markingDeskHtml(): string {
  const status = String(task?.status || "");
  const closed = status === "closed";
  const graded = status === "graded";
  const released = status === "released";
  const row = markingRows().find((r) => String(r.attempt_id) === gradeStu);
  const nextId = nextPendingAttempt(gradeStu);
  const who = row ? `${esc(row.name)} · ${esc(row.student_no)}` : "给分台";
  const codes = ((row?.code_items as Array<Record<string, unknown>>) || []);
  const confirmed = Boolean(row?.marking_confirmed);
  const canConfirm = Boolean(gradeStu && codes.length && !codePending(row || {}) && !confirmed && markingCanScore());
  const ended = closed || graded;
  const allDone = !markingRows().some(markingPending);
  const canRelease = ended && allDone && !released;
  let markHint = `${clipTitle(task?.title)} · 点色块即给分。给完该生待批改题后点提交，确认这份答卷。`;
  if (status === "published") {
    markHint += "全部确认并结束任务后可公开试卷。";
  } else if (!allDone) {
    markHint += "还有学生未确认批改。";
  } else if (ended) {
    markHint += "可以公开试卷。";
  } else {
    markHint += "结束任务后可公开试卷。";
  }
  return `<div class="head-row">
      <h2 class="mark-stu">${who}</h2>
      <div class="acts">
        ${!released && codes.length && markingCanScore() ? `<button class="primary" type="button" data-click="confirm-mark:${esc(gradeStu)}"${canConfirm ? "" : " disabled"}>提交</button>` : ""}
        ${released ? "" : `<button type="button" data-click="release:${esc(taskId)}"${canRelease ? "" : " disabled"}>公开试卷</button>`}
        ${gradeStu ? `<button class="primary" type="button" data-click="view-paper:${esc(gradeStu)}">查看试卷</button>` : ""}
        ${nextId ? `<button type="button" data-mark-next="${esc(nextId)}">下一位</button>` : ""}
      </div>
    </div>
    <p class="hint">${markHint}</p>
    <p class="toolbar"><label class="check early-flag"><input type="checkbox" data-auto-grade ${task?.auto_grade !== false ? "checked" : ""}${released ? " disabled" : ""} /> <span>自动批改填空题</span></label></p>
    ${markingCodeHtml()}`;
}

function markingCodeHtml(): string {
  if (!gradeStu) {
    return `<div class="code-mark"><p class="hint">从中间栏选择一名考生。</p></div>`;
  }
  const row = markingRows().find((r) => String(r.attempt_id) === gradeStu);
  if (!row) {
    return `<div class="code-mark"><p class="hint">从中间栏选择一名考生。</p></div>`;
  }
  const codes = (row.code_items as Array<Record<string, unknown>>) || [];
  if (!codes.length) {
    return `<div class="code-mark"><p class="hint">该生没有需要手工给分的题目。可点「查看试卷」批阅整卷。</p></div>`;
  }
  const locked = !markingCanScore();
  const attempt = String(row.attempt_id || "");
  const body = codes
    .map((c, i) => {
      const scored = c.score == null || c.score === "" ? "未给分" : `已记 ${esc(c.score)} 分`;
      return `<article class="code-item">
          <h3>第 ${i + 1} 题 · ${esc(qlabel(String(c.qtype || "code")))} · 满分 ${esc(c.points)} <span class="muted">${scored}</span></h3>
          <p class="code-stem">${esc(c.stem)}</p>
          <pre>${esc(codeResponseText(c))}</pre>
          ${scorePadsHtml(attempt, c.question_id, c.points, c.score, locked)}
        </article>`;
    })
    .join("");
  return `<div class="code-mark">${body}</div>`;
}

function gradesHtml(): string {
  const gradeTasks = tasks.filter((t) => String(t.status) !== "draft");
  const list = gradeTasks
    .map(
      (t) =>
        `<tr class="pick${taskId === t.id ? " on" : ""}" data-task="${esc(t.id)}"><td class="clip">${clipTitle(t.title)}</td>
         <td class="st"><span class="badge ${badgeFor(t.status)}">${esc(statusLabel(t.status))}</span></td>
         <td class="num">${esc(t.total_score)}</td></tr>`,
    )
    .join("");
  let right = `<h2>成绩总览</h2><p class="hint">选择左侧任务查看当前交卷数据。</p>`;
  if (task && String(task.status) !== "draft") {
    right = gradeRightHtml();
  }
  return `<div class="grid grades">
    <section class="panel">
      <h2>考试任务</h2>
      <div class="scroll"><table class="task-list"><thead><tr><th class="clip">标题</th><th class="st">状态</th><th class="num">总分</th></tr></thead>
      <tbody>${list || `<tr><td colspan="3" class="muted">还没有可分析的任务</td></tr>`}</tbody></table></div>
    </section>
    <section class="panel">${right}</section>
  </div>${gradeQuestionDialogHtml()}${gradePaperDialogHtml()}`;
}

function gradeRightHtml(): string {
  const pane = (id: "board" | "students" | "questions", lab: string) =>
    `<button type="button" data-grade-pane="${id}" class="${gradePane === id ? "on" : ""}">${lab}</button>`;
  const body =
    gradePane === "students" ? gradeStudentsHtml() : gradePane === "questions" ? gradeQuestionsHtml() : gradeBoardHtml();
  return `<div class="head-row">
      <h2>${clipTitle(task?.title)}</h2>
      <div class="acts">
        ${gradePane === "students" && gradeStu ? `<button class="primary" type="button" data-click="view-paper:${esc(gradeStu)}">查看试卷</button>` : ""}
        <button type="button" data-click="csv:${esc(taskId)}">导出成绩</button>
      </div>
    </div>
    <div class="seg grade-seg">${pane("board", "数据总览")}${pane("students", "学生成绩")}${pane("questions", "题目分析")}</div>
    ${body}`;
}

function gradeBoardHtml(): string {
  const a = analysis || {};
  const bandInfo = (a.bands as {
    cuts?: number[];
    labels?: string[];
    counts?: number[];
    below_60?: number;
    "60_70"?: number;
    "70_80"?: number;
    "80_90"?: number;
    "90_95"?: number;
    "95_100"?: number;
  }) || {};
  const cuts = (analysisBands ? analysisBands.split(",") : (bandInfo.cuts || []).map(String)).slice(0, 5);
  while (cuts.length < 5) {
    cuts.push("");
  }
  const bandItems: Array<[string, number]> =
    (bandInfo.labels || []).length > 0
      ? (bandInfo.labels || []).map((lab, i) => [lab, Number((bandInfo.counts || [])[i] || 0)])
      : [
          ["<60%", Number(bandInfo.below_60 || 0)],
          ["60-70", Number(bandInfo["60_70"] || 0)],
          ["70-80", Number(bandInfo["70_80"] || 0)],
          ["80-90", Number(bandInfo["80_90"] || 0)],
          ["90-95", Number(bandInfo["90_95"] || 0)],
          ["95-100", Number(bandInfo["95_100"] || 0)],
        ];
  const qn = Number(a.question_count ?? ((a.questions as Array<Record<string, unknown>>) || []).length);
  const types = ((a.type_rates as Array<Record<string, unknown>>) || []).map(
    (t): [string, number, string] => [qlabel(String(t.qtype)), Number(t.score_rate), `${fmtPct(t.score_rate)} · ${t.count}题`],
  );
  const classes = ((a.classes as Array<Record<string, unknown>>) || []).map(
    (c): [string, number, string] => [String(c.class_name || "未分班"), Number(c.pass_rate), `${fmtScore(c.average)}分 · ${fmtPct(c.pass_rate)}`],
  );
  const topRows = (a.top_students as Array<Record<string, unknown>>) || [];
  const bottomRows = (a.bottom_students as Array<Record<string, unknown>>) || [];
  return `<div class="dash">
    <form name="analysis" class="row analysis-form">
      <label class="field compact">及格分<input name="pass_score" type="number" value="${esc(analysisPass || a.pass_score)}" /></label>
      <label class="field compact">良好分<input name="good_score" type="number" value="${esc(analysisGood || a.good_score)}" /></label>
      <label class="field compact">优秀分<input name="excellent_score" type="number" value="${esc(analysisExcellent || a.excellent_score)}" /></label>
      <label class="field compact">前列%<input name="top_pct" type="number" min="1" max="100" step="0.1" value="${esc(analysisTopPct || a.top_pct)}" /></label>
      <label class="field compact">后列%<input name="bottom_pct" type="number" min="1" max="100" step="0.1" value="${esc(analysisBottomPct || a.bottom_pct)}" /></label>
      <label class="field band-field"><span>分数段分界</span><span class="band-cuts">
        <input name="band1" type="number" value="${esc(cuts[0])}" />
        <input name="band2" type="number" value="${esc(cuts[1])}" />
        <input name="band3" type="number" value="${esc(cuts[2])}" />
        <input name="band4" type="number" value="${esc(cuts[3])}" />
        <input name="band5" type="number" value="${esc(cuts[4])}" />
      </span></label>
      <button type="submit">重算分析</button>
    </form>
    <div class="stats dash">
      ${gradeStat("已交卷", String(a.count ?? 0), "students", `交卷率 ${fmtPct(a.submit_rate)}`)}
      ${gradeStat("平均分", fmtScore(a.average), "students", fmtPct(a.avg_rate))}
      ${gradeStat("及格率", fmtPct(a.pass_rate), "students", `及格线 ${fmtScore(a.pass_score, 0)}`)}
      ${gradeStat("优秀率", fmtPct(a.excellent_rate), "students", `≥${fmtScore(a.excellent_score, 0)}`)}
      ${gradeStat("优秀+率", fmtPct(a.excellent_plus_rate), "students", `≥${fmtScore(a.excellent_plus_score, 0)}`)}
      ${gradeStat("良好率", fmtPct(a.good_rate), "students", `≥${fmtScore(a.good_score, 0)}`)}
      ${gradeStat("满分率", fmtPct(a.full_rate), "students", `${esc(a.full_mark ?? 0)}人`)}
      ${gradeStat("最高 / 最低", `${fmtScore(a.max, 0)} / ${fmtScore(a.min, 0)}`)}
      ${gradeStat("中位 / 标准差", `${fmtScore(a.median)} / ${fmtScore(a.stddev)}`)}
      ${gradeStat("变异系数", fmtPct(a.cv), "", `四分位 ${fmtScore(a.q1)} ~ ${fmtScore(a.q3)}`)}
      ${gradeStat("题目数", String(qn), "questions")}
    </div>
    <div class="charts">
      ${vbarChartHtml("分数段人数", bandItems)}
      ${donutChartHtml("成绩构成", [
        ["优秀+", Number(a.excellent_plus || 0), "#7fb80e"],
        ["优秀", Number(a.excellent_only || 0), "#005831"],
        ["良好", Number(a.good_only || 0), "#33a3dc"],
        ["及格", Number(a.pass_only || 0), "#ffc20e"],
        ["不及格", Number(a.failed || 0), "#aa2116"],
      ])}
      ${hbarChartHtml("题型得分率", types, "questions")}
      ${hbarChartHtml("班级及格率", classes, "students")}
    </div>
    <div class="rank-pair">
      ${rankTableHtml(`前列 ${fmtPctNum(a.top_pct)} · ${esc(a.top_n ?? topRows.length)}人`, topRows)}
      ${rankTableHtml(`后列 ${fmtPctNum(a.bottom_pct)} · ${esc(a.bottom_n ?? bottomRows.length)}人`, bottomRows)}
    </div>
  </div>`;
}

function gradeStudentsHtml(): string {
  const rows = ((results?.results as Array<Record<string, unknown>>) || [])
    .map((r) => {
      const picked = gradeStu === String(r.attempt_id);
      return `<tr class="pick${picked ? " on" : ""}" data-grade-stu="${esc(r.attempt_id)}">
        <td class="mono">${esc(r.student_no)}</td><td>${esc(r.name)}</td><td>${esc(r.class_name)}</td>
        <td>${esc(r.total_score)}</td><td>${esc(r.submitted_at || "—")}</td>
        <td>${esc((r.item_scores as unknown[])?.join(" / "))}</td>
      </tr>`;
    })
    .join("");
  return `<div class="scroll grade-detail"><p class="hint">选中一名学生后，点标题栏「查看试卷」。</p><table>
    <thead><tr><th>学号</th><th>姓名</th><th>班级</th><th>总分</th><th>提交时间</th><th>各题</th></tr></thead>
    <tbody>${rows || `<tr><td colspan="6" class="muted">还没有交卷记录</td></tr>`}</tbody>
  </table></div>`;
}

function gradeQuestionsHtml(): string {
  if (analysis?.live_draw) {
    return `<div class="scroll grade-detail"><p class="hint">这份任务使用智能实时组卷，每位考生题目不同，题目分析没有对照意义。</p></div>`;
  }
  const qs = (analysis?.questions as Array<Record<string, unknown>>) || [];
  const groups = new Map<string, Array<Record<string, unknown>>>();
  for (const q of qs) {
    const t = String(q.qtype || "");
    const g = groups.get(t) || [];
    g.push(q);
    groups.set(t, g);
  }
  let i = 0;
  const secs = Array.from(groups.entries())
    .map(([t, items]) => {
      i += 1;
      const rows = items
        .map(
          (q) => `<tr>
            <td><button type="button" class="qnum" data-grade-q="${esc(q.index)}">${esc(q.index)}</button></td>
            <td>${esc(qlabel(t))}</td>
            <td>${fmtPct(q.correct_rate)}</td>
            <td>${fmtPct(q.score_rate)}</td>
            <td>${fmtScore(q.avg_score)} / ${esc(q.points)}</td>
          </tr>`,
        )
        .join("");
      const st = sectionAgg(items);
      return `<details class="qsec"${i === 1 ? " open" : ""}>
        <summary>${esc(typeSection(t))} · ${items.length} 题 <span class="qsec-meta">正确率 ${fmtPct(st.cr)} · 得分率 ${fmtPct(st.sr)} · 均分 ${fmtScore(st.avg)} / ${esc(st.pts)}</span></summary>
        <table><thead><tr><th>题号</th><th>题型</th><th>正确率</th><th>得分率</th><th>均分</th></tr></thead>
        <tbody>${rows}</tbody></table>
      </details>`;
    })
    .join("");
  return `<div class="scroll grade-detail">${secs || `<p class="hint">暂无题目分析。</p>`}</div>`;
}

function gradeQuestionDialogHtml(): string {
  if (!gradeQ) {
    return "";
  }
  const q = gradeQ;
  const opts = ((q.options as Array<{ key?: string; text?: string }>) || [])
    .map((o) => `${esc(o.key)}. ${esc(o.text)}`)
    .join("　");
  const stem = displayStemHtml(q.stem_html, q.stem, !!opts);
  return `<div class="modal" data-grade-q-close>
    <div class="dialog">
      <div class="head-row">
        <h2>第 ${esc(q.index)} 题 · ${esc(qlabel(String(q.qtype)))}</h2>
        <button type="button" class="ghost" data-grade-q-close>关闭</button>
      </div>
      <div class="qsheet"><article class="qitem on"><div class="question">
        ${stem}
        ${opts ? `<div class="options">${opts}</div>` : ""}
        <div class="answer">${formatAnswerHtml(q)}</div>
      </div></article></div>
      <p class="hint">正确率 ${fmtPct(q.correct_rate)} · 得分率 ${fmtPct(q.score_rate)} · 均分 ${fmtScore(q.avg_score)} / ${esc(q.points)} · ${esc(q.n)} 人作答</p>
    </div>
  </div>`;
}

function itemCorrect(item: Record<string, unknown>): boolean {
  return item.correct === true || (Array.isArray(item.correct) && item.correct[0] === true);
}

function gradePaperDialogHtml(): string {
  if (!gradePaper) {
    return "";
  }
  const p = gradePaper;
  const items = (p.items as Array<Record<string, unknown>>) || [];
  const rows = items
    .map((it, i) => {
      const ok = itemCorrect(it);
      const mark = ok ? '<span class="ok">正确</span>' : '<span class="bad">待核 / 错误</span>';
      const hasOpts = Array.isArray(it.options) && (it.options as unknown[]).length > 0;
      const stem = displayStemHtml(it.stem_html, it.stem, hasOpts);
      return `<article class="qitem">
        <h3>${i + 1}. [${esc(qlabel(String(it.qtype)))}] ${mark} ${esc(it.score ?? "—")} / ${esc(it.points ?? "")}</h3>
        <div class="question">${stem}${reviewOptionsHtml(it)}
          <p>作答：${esc(showAns(it.response))}</p>
          <p>参考：${esc(showAns(it.answer))}</p>
        </div>
      </article>`;
    })
    .join("");
  return `<div class="modal" data-grade-paper-close>
    <div class="dialog wide">
      <div class="head-row">
        <h2>${esc(p.name)} · ${esc(p.student_no)} · ${esc(p.class_name)}</h2>
        <button type="button" class="ghost" data-grade-paper-close>关闭</button>
      </div>
      <p class="hint">${clipTitle(p.title)} · 总分 ${esc(p.total_score)} / ${esc(p.paper_total)} · 客观 ${esc(p.objective_score)}</p>
      <div class="qsheet paper-review">${rows || `<p class="hint">没有题目。</p>`}</div>
    </div>
  </div>`;
}

function startMonitor(): void {
  if (monitorTimer) {
    window.clearInterval(monitorTimer);
    monitorTimer = null;
  }
  if (token && tab === "tasks" && taskId && task && (String(task.status) === "published" || isHistory(task.status))) {
    monitorTimer = window.setInterval(async () => {
      const m = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/monitor`);
      if (m.status === 200) {
        monitor = m.data;
        const el = document.getElementById("monitor-mount");
        if (el) {
          el.innerHTML = monitorInner();
          el.querySelectorAll("[data-page]").forEach((btn) => {
            btn.addEventListener("click", () => {
              if ((btn as HTMLButtonElement).disabled) {
                return;
              }
              const dir = ((btn as HTMLElement).dataset.page || "").split(":")[1];
              monPage = Math.max(1, monPage + (dir === "next" ? 1 : -1));
              render();
            });
          });
        }
      }
    }, 5000);
  }
}

function render(): void {
  if (!token) {
    if (monitorTimer) {
      window.clearInterval(monitorTimer);
      monitorTimer = null;
    }
    root.innerHTML = gateHtml();
    bind();
    return;
  }
  const [title, sub] = titles()[tab];
  const body =
    tab === "banks"
        ? banksHtml()
        : tab === "roster"
          ? rosterHtml()
          : tab === "papers"
            ? papersHtml()
            : tab === "tasks"
              ? tasksHtml()
              : tab === "marking"
                ? markingHtml()
                : gradesHtml();
  root.innerHTML = `<div class="shell">
    <aside class="rail">
      <div>
        <div class="kicker">控制室</div>
        <div class="brand-name">讲台</div>
        <div class="brand-sub">局域网考场</div>
      </div>
      ${navHtml()}
      <div class="rail-foot"><button type="button" id="logout">离开讲台</button></div>
    </aside>
    <main class="stage">
      <header class="stage-head">
        <div><h1>${title}</h1><p class="hint">${sub}</p></div>
      </header>
      <div class="workspace${tab === "banks" || tab === "roster" || tab === "papers" || tab === "tasks" || tab === "marking" || tab === "grades" ? " fit" : ""}">${body}</div>
    </main>
  </div>`;
  bind();
  startMonitor();
  fitVisibleLists();
}

void loadTab().then(render);

let resizeTimer = 0;
window.addEventListener("resize", () => {
  window.clearTimeout(resizeTimer);
  resizeTimer = window.setTimeout(() => {
    if (token && !showPaperForm && !showQForm && !showTaskForm && !scoreEditing()) {
      paperNeedPack = true;
      bankNeedPack = true;
      render();
    }
  }, 120);
});

window.addEventListener("keydown", (ev) => {
    if ((ev as KeyboardEvent).key === "Escape" && notice) {
      hideNotice();
    }
  });
