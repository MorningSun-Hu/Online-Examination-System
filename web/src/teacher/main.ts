import { api, errMsg } from "../shared/api";

type Tab = "access" | "banks" | "roster" | "tasks" | "grades";

const TOKEN_KEY = "exam_teacher_token";
const QTYPES: [string, string][] = [
  ["single", "单选"],
  ["multiple", "多选"],
  ["judge", "判断"],
  ["fill", "填空"],
  ["code", "编程"],
];

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function qlabel(t: string): string {
  return QTYPES.find(([k]) => k === t)?.[1] || t;
}

let token = localStorage.getItem(TOKEN_KEY);
let tab: Tab = "access";
let flash = "";
let lan: { addresses?: string[]; port?: number } | null = null;
let banks: Array<Record<string, unknown>> = [];
let bankId = "";
let bank: Record<string, unknown> | null = null;
let students: Array<Record<string, unknown>> = [];
let tasks: Array<Record<string, unknown>> = [];
let taskId = "";
let task: Record<string, unknown> | null = null;
let roster: Record<string, unknown> | null = null;
let monitor: Record<string, unknown> | null = null;
let results: Record<string, unknown> | null = null;
let analysis: Record<string, unknown> | null = null;
let monitorTimer: number | null = null;

const root = document.getElementById("app")!;

async function tcall<T = Record<string, unknown>>(path: string, opts: RequestInit = {}) {
  const r = await api<T>(path, { ...opts, token });
  if (r.status === 401) {
    token = null;
    localStorage.removeItem(TOKEN_KEY);
    flash = "登录已过期，请重新进入讲台";
    render();
  }
  return r;
}

function bodyJson(v: unknown): RequestInit {
  return { method: "POST", body: JSON.stringify(v) };
}

async function loadTab(): Promise<void> {
  if (!token) {
    return;
  }
  if (tab === "access") {
    const r = await tcall<{ addresses: string[]; port: number }>("/api/teacher/lan-addresses");
    if (r.status === 200) {
      lan = r.data;
    }
  }
  if (tab === "banks") {
    const r = await tcall<{ banks: Array<Record<string, unknown>> }>("/api/teacher/banks");
    if (r.status === 200) {
      banks = r.data.banks || [];
    }
    if (bankId) {
      const d = await tcall<Record<string, unknown>>(`/api/teacher/banks/${bankId}`);
      bank = d.status === 200 ? d.data : null;
    }
  }
  if (tab === "roster") {
    const r = await tcall<{ students: Array<Record<string, unknown>> }>("/api/teacher/students");
    if (r.status === 200) {
      students = r.data.students || [];
    }
  }
  if (tab === "tasks" || tab === "grades") {
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
    if (taskId) {
      await loadTask();
    }
  }
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
  if (tab === "grades") {
    const rs = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/results`);
    results = rs.status === 200 ? rs.data : null;
    const an = await tcall<Record<string, unknown>>(`/api/teacher/tasks/${taskId}/analysis`);
    analysis = an.status === 200 ? an.data : null;
  }
}

function bind(): void {
  root.querySelectorAll("[data-tab]").forEach((el) => {
    el.addEventListener("click", async () => {
      tab = (el as HTMLElement).dataset.tab as Tab;
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
      await loadTab();
      render();
    });
  });
  root.querySelectorAll("[data-task]").forEach((el) => {
    el.addEventListener("click", async () => {
      taskId = (el as HTMLElement).dataset.task || "";
      await loadTask();
      render();
    });
  });
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
        flash = errMsg(r.data, "口令校验失败");
        render();
        return;
      }
      token = (r.data as { token: string }).token;
      localStorage.setItem(TOKEN_KEY, token);
      flash = "讲台已解锁";
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
      flash = r.status === 204 ? "口令已更新" : errMsg(r.data);
    }
    if (name === "bank-create") {
      const r = await tcall("/api/teacher/banks", bodyJson({ name: val("name") }));
      flash = r.status === 201 ? "题库已创建" : errMsg(r.data);
    }
    if (name === "bank-import") {
      const file = fd.get("file") as File | null;
      const html = file ? await file.text() : "";
      const r = await tcall(`/api/teacher/banks/${bankId}/import-html`, {
        method: "POST",
        body: JSON.stringify({ filename: file?.name || "upload.html", html }),
      });
      flash =
        r.status < 400
          ? `导入成功 ${(r.data as { imported?: number }).imported || 0} 题，失败 ${(r.data as { failed?: number }).failed || 0}`
          : errMsg(r.data);
    }
    if (name === "question") {
      const qtype = val("qtype");
      const options = String(fd.get("options") || "")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          const [key, ...rest] = line.split("|");
          return { key: key.trim(), text: rest.join("|").trim() || key.trim() };
        });
      const rawAns = val("answer");
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
      flash = r.status < 400 ? "题目已保存" : errMsg(r.data);
    }
    if (name === "csv") {
      const file = fd.get("file") as File | null;
      const csv = file ? await file.text() : val("csv");
      const r = await tcall("/api/teacher/students/import-csv", {
        method: "POST",
        body: JSON.stringify({ filename: file?.name || "students.csv", csv }),
      });
      flash =
        r.status < 400
          ? `名册写入 ${(r.data as { upserted?: number }).upserted || 0} 人，跳过 ${(r.data as { failed?: number }).failed || 0} 行`
          : errMsg(r.data);
    }
    if (name === "task-create") {
      const type_counts: Record<string, number> = {};
      for (const [k] of QTYPES) {
        const n = Number(fd.get(`count_${k}`) || 0);
        if (n > 0) {
          type_counts[k] = n;
        }
      }
      const student_ids = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=student_id]:checked")).map(
        (i) => i.value,
      );
      const bank_ids = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=bank_id]:checked")).map((i) => i.value);
      const r = await tcall("/api/teacher/tasks", {
        method: "POST",
        body: JSON.stringify({
          title: val("title"),
          mode: val("mode"),
          duration_minutes: Number(val("duration_minutes") || 60),
          allow_early_submit: fd.get("allow_early_submit") === "on",
          total_score: Number(val("total_score") || 100),
          pass_score: val("pass_score") ? Number(val("pass_score")) : undefined,
          student_ids,
          rule: {
            bank_ids,
            type_counts,
            shuffle_within_section: fd.get("shuffle_within_section") === "on",
            shuffle_options: fd.get("shuffle_options") === "on",
          },
        }),
      });
      if (r.status === 201) {
        taskId = String((r.data as { id: string }).id);
        flash = "任务已创建，请核对分值后发布";
      } else if (r.status === 409) {
        const details = (r.data as { error?: { details?: { task_id?: string }; message?: string } }).error;
        taskId = details?.details?.task_id || taskId;
        flash = details?.message || errMsg(r.data);
      } else {
        flash = errMsg(r.data);
      }
    }
    if (name === "roster") {
      const student_ids = Array.from(form.querySelectorAll<HTMLInputElement>("input[name=student_id]:checked")).map(
        (i) => i.value,
      );
      const r = await tcall(`/api/teacher/tasks/${taskId}/roster`, {
        method: "PUT",
        body: JSON.stringify({ student_ids }),
      });
      flash = r.status === 204 ? "名单已更新" : errMsg(r.data);
    }
    if (name === "early") {
      const r = await tcall(`/api/teacher/tasks/${taskId}`, {
        method: "PATCH",
        body: JSON.stringify({ allow_early_submit: fd.get("allow_early_submit") === "on" }),
      });
      flash = r.status === 204 ? "提前交卷开关已更新" : errMsg(r.data);
    }
    if (name === "code-score") {
      const attempt = val("attempt_id");
      const scores: Record<string, number> = {};
      form.querySelectorAll<HTMLInputElement>("input[data-qid]").forEach((inp) => {
        if (inp.value) {
          scores[inp.dataset.qid || ""] = Number(inp.value);
        }
      });
      const r = await tcall(`/api/teacher/attempts/${attempt}/code-scores`, {
        method: "PUT",
        body: JSON.stringify({ scores }),
      });
      flash = r.status < 400 ? "编程题分数已写入" : errMsg(r.data);
    }
    if (name === "analysis") {
      const q = val("pass_score");
      const r = await tcall(`/api/teacher/tasks/${taskId}/analysis${q ? `?pass_score=${q}` : ""}`);
      analysis = r.status === 200 ? r.data : analysis;
      flash = r.status === 200 ? "分析已刷新" : errMsg(r.data);
    }
    await loadTab();
    render();
  } catch (e) {
    flash = String(e);
    render();
  }
}

async function act(kind: string, id?: string): Promise<void> {
  const map: Record<string, [string, string]> = {
    publish: ["POST", `/api/teacher/tasks/${id}/publish`],
    close: ["POST", `/api/teacher/tasks/${id}/close`],
    grade: ["POST", `/api/teacher/tasks/${id}/grade`],
    release: ["POST", `/api/teacher/tasks/${id}/release`],
  };
  if (kind === "del-bank") {
    const r = await tcall(`/api/teacher/banks/${id}`, { method: "DELETE" });
    flash = r.status === 204 ? "题库已删除" : errMsg(r.data);
    bankId = "";
  } else if (kind === "del-q") {
    const r = await tcall(`/api/teacher/questions/${id}`, { method: "DELETE" });
    flash = r.status === 204 ? "题目已删除" : errMsg(r.data);
  } else if (kind === "export") {
    const res = await fetch(`/api/teacher/banks/${id}/export-html`, {
      headers: { authorization: `Bearer ${token}` },
    });
    const blob = await res.blob();
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "bank.html";
    a.click();
    return;
  } else if (kind === "csv") {
    const res = await fetch(`/api/teacher/tasks/${id}/results.csv`, {
      headers: { authorization: `Bearer ${token}` },
    });
    const blob = await res.blob();
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "results.csv";
    a.click();
    return;
  } else if (map[kind]) {
    const [method, path] = map[kind];
    const r = await tcall(path, { method, body: method === "POST" ? "{}" : undefined });
    flash = r.status < 400 ? "操作完成" : errMsg(r.data);
  }
  await loadTab();
  render();
}

function gateHtml(): string {
  return `<div class="gate">
    <div class="kicker">Classroom Console</div>
    <h1>讲台口令</h1>
    <p>第一次进入会把口令写进本机题库，之后用同一口令解锁。</p>
    <form name="gate">
      <label>教师口令<input name="password" type="password" required /></label>
      <button class="primary" type="submit">进入讲台</button>
    </form>
    ${flash ? `<div class="flash">${esc(flash)}</div>` : ""}
  </div>`;
}

function navHtml(): string {
  const items: [Tab, string][] = [
    ["access", "接入"],
    ["banks", "题库"],
    ["roster", "名册"],
    ["tasks", "任务"],
    ["grades", "成绩"],
  ];
  return `<nav>${items
    .map(([k, lab]) => `<button data-tab="${k}" class="${tab === k ? "active" : ""}">${lab}</button>`)
    .join("")}<button id="logout">离开</button></nav>`;
}

function accessHtml(): string {
  const port = lan?.port || 8080;
  const links = (lan?.addresses || []).map((ip) => {
    const url = `http://${ip}:${port}/student.html`;
    return `<p class="lan"><a href="${esc(url)}" target="_blank">${esc(url)}</a></p>`;
  });
  return `<section class="panel">
    <h2>学生入口</h2>
    <p class="muted">把下面地址发给学生机浏览器。本页只列出非回环 IPv4。</p>
    ${links.join("") || "<p>暂无局域网地址</p>"}
    <form name="password" class="row">
      <label>当前口令<input name="current_password" type="password" required /></label>
      <label>新口令<input name="new_password" type="password" required /></label>
      <button class="primary" type="submit">改口令</button>
    </form>
  </section>`;
}

function banksHtml(): string {
  const list = banks
    .map(
      (b) => `<tr>
      <td><button data-bank="${esc(b.id)}">${esc(b.name)}</button></td>
      <td>${esc(b.question_count)}</td>
      <td><button data-click="export:${esc(b.id)}">导出</button>
          <button class="danger" data-click="del-bank:${esc(b.id)}">删除</button></td>
    </tr>`,
    )
    .join("");
  const qs = ((bank?.questions as Array<Record<string, unknown>>) || [])
    .map((q) => {
      const ans = JSON.stringify(q.answer);
      return `<div class="q">
        <strong>${esc(qlabel(String(q.qtype)))}</strong> ${esc(q.stem)}
        <div class="muted">${esc(ans)}</div>
        <button class="danger" data-click="del-q:${esc(q.id)}">删除</button>
      </div>`;
    })
    .join("");
  return `<div class="grid">
    <section class="panel">
      <h2>题库</h2>
      <form name="bank-create" class="row">
        <label>名称<input name="name" required /></label>
        <button class="primary" type="submit">新建</button>
      </form>
      <table><thead><tr><th>名称</th><th>题量</th><th></th></tr></thead><tbody>${list}</tbody></table>
    </section>
    <section class="panel">
      <h2>${bank ? esc((bank as { name?: string }).name) : "选择左侧题库"}</h2>
      ${
        bankId
          ? `<form name="bank-import"><label>导入 HTML<input type="file" name="file" accept=".html,text/html" required /></label>
             <button type="submit">导入</button></form>
             <form name="question">
               <input type="hidden" name="qid" />
               <label>题型<select name="qtype">${QTYPES.map(([k, l]) => `<option value="${k}">${l}</option>`).join("")}</select></label>
               <label>题干<textarea name="stem" required></textarea></label>
               <label>选项（每行 键|文本）<textarea name="options" placeholder="A|选项A"></textarea></label>
               <label>答案<input name="answer" placeholder="单选填 A；多选 A,C；判断 true；填空 a|b" /></label>
               <label>来源<input name="source" /></label>
               <button class="primary" type="submit">保存题目</button>
             </form>${qs}`
          : ""
      }
    </section>
  </div>`;
}

function rosterHtml(): string {
  const rows = students
    .map((s) => `<tr><td>${esc(s.class_name)}</td><td class="mono">${esc(s.student_no)}</td><td>${esc(s.name)}</td></tr>`)
    .join("");
  return `<section class="panel">
    <h2>学生名册</h2>
    <form name="csv">
      <label>CSV 文件（表头需含班级、学号、姓名）<input type="file" name="file" accept=".csv,text/csv" /></label>
      <label>或直接粘贴<textarea name="csv" rows="6" placeholder="班级,学号,姓名"></textarea></label>
      <button class="primary" type="submit">导入</button>
    </form>
    <table><thead><tr><th>班级</th><th>学号</th><th>姓名</th></tr></thead><tbody>${rows}</tbody></table>
  </section>`;
}

function planHtml(): string {
  const plan = task?.score_plan as Record<string, number[]> | undefined;
  if (!plan) {
    return "<p class=\"muted\">尚无分值方案</p>";
  }
  return Object.entries(plan)
    .map(([k, arr]) => `<p>${esc(qlabel(k))}：${arr.join(" + ")} = ${arr.reduce((a, b) => a + b, 0)}</p>`)
    .join("");
}

function tasksHtml(): string {
  const list = tasks
    .map(
      (t) =>
        `<tr><td><button data-task="${esc(t.id)}">${esc(t.title)}</button></td><td>${esc(t.mode)}</td><td>${esc(t.status)}</td><td>${esc(t.total_score)}</td></tr>`,
    )
    .join("");
  const bankChecks = banks
    .map((b) => `<label><input type="checkbox" name="bank_id" value="${esc(b.id)}" /> ${esc(b.name)}</label>`)
    .join("");
  const stuChecks = students
    .map((s) => {
      const ids = ((roster?.student_ids as string[]) || []);
      const checked = ids.includes(String(s.id)) ? "checked" : "";
      return `<label><input type="checkbox" name="student_id" value="${esc(s.id)}" ${checked} /> ${esc(s.class_name)} ${esc(s.student_no)} ${esc(s.name)}</label>`;
    })
    .join("");
  const counts = QTYPES.map(
    ([k, l]) => `<label>${l}题量<input type="number" min="0" name="count_${k}" value="0" /></label>`,
  ).join("");
  const mon = ((monitor?.students as Array<Record<string, unknown>>) || [])
    .map((s) => `<tr><td>${esc(s.class_name)}</td><td>${esc(s.student_no)}</td><td>${esc(s.name)}</td><td>${esc(s.status)}</td></tr>`)
    .join("");
  const early = task?.allow_early_submit ? "checked" : "";
  return `<div class="grid">
    <section class="panel">
      <h2>任务列表</h2>
      <table><tbody>${list}</tbody></table>
      <form name="task-create">
        <label>标题<input name="title" required /></label>
        <label>模式<select name="mode"><option value="exam">考试</option><option value="practice">练习</option></select></label>
        <label>时长（分钟）<input name="duration_minutes" type="number" value="60" /></label>
        <label>总分<input name="total_score" type="number" value="100" /></label>
        <label>及格分<input name="pass_score" type="number" /></label>
        <label><input type="checkbox" name="allow_early_submit" /> 允许提前交卷</label>
        <label><input type="checkbox" name="shuffle_within_section" checked /> 题型内乱序</label>
        <label><input type="checkbox" name="shuffle_options" checked /> 选项乱序</label>
        <h3>抽题</h3>${counts}${bankChecks}
        <h3>名单</h3>${stuChecks}
        <button class="primary" type="submit">生成任务</button>
      </form>
    </section>
    <section class="panel">
      <h2>${task ? esc(task.title) : "任务详情"}</h2>
      ${task ? `<p>${esc(task.mode)} / ${esc(task.status)} / 关闭于 ${esc(task.closes_at || "—")}</p>
        <h3>分值确认</h3>${planHtml()}
        <p>
          <button data-click="publish:${esc(taskId)}">发布</button>
          <button data-click="close:${esc(taskId)}">收卷</button>
        </p>
        <form name="early"><label><input type="checkbox" name="allow_early_submit" ${early} /> 允许提前交卷</label>
          <button type="submit">更新开关</button></form>
        <form name="roster"><h3>任务名单</h3>${stuChecks}<button type="submit">保存名单</button></form>
        <h3>监控</h3>
        <table><thead><tr><th>班级</th><th>学号</th><th>姓名</th><th>状态</th></tr></thead><tbody>${mon}</tbody></table>` : "<p>选择左侧任务</p>"}
    </section>
  </div>`;
}

function gradesHtml(): string {
  const list = tasks
    .map((t) => `<button data-task="${esc(t.id)}">${esc(t.title)} · ${esc(t.status)}</button>`)
    .join(" ");
  const rows = ((results?.results as Array<Record<string, unknown>>) || [])
    .map((r) => {
      const codes = ((r.code_items as Array<Record<string, unknown>>) || [])
        .map(
          (c) =>
            `<div class="muted">${esc(c.stem)}（满分 ${esc(c.points)}）<pre>${esc(JSON.stringify(c.response))}</pre>
             <label>给分<input data-qid="${esc(c.question_id)}" type="number" min="0" max="${esc(c.points)}" value="${esc(c.score ?? "")}" /></label>`,
        )
        .join("");
      return `<tr>
        <td>${esc(r.student_no)}</td><td>${esc(r.name)}</td><td>${esc(r.total_score)}</td>
        <td class="mono">${esc(r.client_id)}</td><td>${esc(r.submitted_at)}</td>
        <td>${esc((r.item_scores as unknown[])?.join(" / "))}</td>
      </tr>
      ${
        codes
          ? `<tr><td colspan="6"><form name="code-score"><input type="hidden" name="attempt_id" value="${esc(r.attempt_id)}" />${codes}<button type="submit">保存编程分</button></form></td></tr>`
          : ""
      }`;
    })
    .join("");
  const a = analysis || {};
  const bands = (a.bands as Record<string, number>) || {};
  const qs = ((a.questions as Array<Record<string, unknown>>) || [])
    .map((q) => `<tr><td>${esc(q.index)}</td><td>${esc(qlabel(String(q.qtype)))}</td><td>${esc(q.correct_rate)}</td><td>${esc(q.score_rate)}</td></tr>`)
    .join("");
  return `<section class="panel">
    <h2>成绩与回发</h2>
    <p>${list}</p>
    ${
      task
        ? `<p>
        <button data-click="grade:${esc(taskId)}">确认批改</button>
        <button data-click="release:${esc(taskId)}">公开试卷</button>
        <button data-click="csv:${esc(taskId)}">导出 CSV</button>
      </p>
      <table><thead><tr><th>学号</th><th>姓名</th><th>总分</th><th>客户端</th><th>提交时间</th><th>各题</th></tr></thead><tbody>${rows}</tbody></table>
      <form name="analysis" class="row">
        <label>及格分<input name="pass_score" type="number" value="${esc(a.pass_score)}" /></label>
        <button type="submit">重算分析</button>
      </form>
      <p>人数 ${esc(a.count)} · 平均 ${esc(a.average)} · 最高 ${esc(a.max)} · 最低 ${esc(a.min)} · 中位 ${esc(a.median)} · 及格率 ${esc(a.pass_rate)}</p>
      <p>分布 &lt;60 ${esc(bands.below_60)} · 60-70 ${esc(bands["60_70"])} · 70-80 ${esc(bands["70_80"])} · 80-90 ${esc(bands["80_90"])} · 90-100 ${esc(bands["90_100"])}</p>
      <table><thead><tr><th>题号</th><th>题型</th><th>正确率</th><th>得分率</th></tr></thead><tbody>${qs}</tbody></table>`
        : "<p>选择任务</p>"
    }
  </section>`;
}

function render(): void {
  if (monitorTimer) {
    window.clearInterval(monitorTimer);
    monitorTimer = null;
  }
  if (!token) {
    root.innerHTML = gateHtml();
    bind();
    return;
  }
  const body =
    tab === "access"
      ? accessHtml()
      : tab === "banks"
        ? banksHtml()
        : tab === "roster"
          ? rosterHtml()
          : tab === "tasks"
            ? tasksHtml()
            : gradesHtml();
  root.innerHTML = `<header><div class="kicker">Teacher Deck</div><h1>讲台控制台</h1></header>
    ${navHtml()}
    ${flash ? `<div class="flash">${esc(flash)}</div>` : ""}
    ${body}`;
  bind();
  root.querySelectorAll("[data-click]").forEach((el) => {
    el.addEventListener("click", () => {
      const [kind, id] = ((el as HTMLElement).dataset.click || "").split(":");
      void act(kind, id);
    });
  });
  if (token && tab === "tasks" && taskId) {
    monitorTimer = window.setInterval(() => {
      void loadTask().then(render);
    }, 5000);
  }
}

void loadTab().then(render);
