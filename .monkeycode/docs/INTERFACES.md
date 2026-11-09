# 接口定义

统一错误体：`{ "error": { "code": "...", "message": "中文说明", "details": {} } }`。

教师接口除 setup/login 外需要 `Authorization` 教师令牌。学生作答接口需要学生会话令牌。`GET /api/health` 返回纯文本 `ok`。

## 公开

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | 健康检查 |
| POST | `/api/teacher/setup` | 首次设口令 |
| POST | `/api/teacher/login` | 教师登录 |
| POST | `/api/student/enter` | 学生进入 |
| GET | `/api/student/brief` | 任务摘要（班级列表、时长、总分） |
| GET | `/api/student/lookup` | 姓名/学号互查 |

根路径 `/`、`/teacher` 307 到 `/teacher.html`；`/student` 307 到 `/student.html`。其余静态资源由 `ServeDir` 提供。

## 教师

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/teacher/password` | 改口令 |
| GET | `/api/teacher/lan-addresses` | 本机非回环 IPv4 |
| GET/POST | `/api/teacher/banks` | 题库列表/创建 |
| GET/PATCH/DELETE | `/api/teacher/banks/{id}` | 题库详情/改名/删除 |
| POST | `/api/teacher/banks/{id}/questions` | 新增题目 |
| PUT/DELETE | `/api/teacher/questions/{id}` | 改题/删题（已被已发布快照引用则 409） |
| POST | `/api/teacher/banks/{id}/import-html` | 导入规范 HTML |
| GET | `/api/teacher/banks/{id}/export-html` | 导出规范 HTML |
| GET/POST | `/api/teacher/students` | 学生列表/登记 |
| POST | `/api/teacher/students/import-csv` | CSV 导入（班级、学号、姓名） |
| PATCH/DELETE | `/api/teacher/students/{id}` | 改学生/删除 |
| GET | `/api/teacher/students.csv` | 导出名册 |
| GET | `/api/teacher/student-template.csv` | 名册模板 |
| GET/POST | `/api/teacher/classes` | 班级列表/创建 |
| PATCH/DELETE | `/api/teacher/classes/{id}` | 改名/删班 |
| GET/POST | `/api/teacher/papers` | 试卷列表/组卷（可带 `auto_grade`，默认开） |
| GET/PATCH/DELETE | `/api/teacher/papers/{id}` | 详情；PATCH 写入 `score_plan` |
| GET | `/api/teacher/papers/{id}/preview` | 预览（预览关闭乱序抽题） |
| GET/POST | `/api/teacher/tasks` | 任务列表/创建（优先 `paper_id`，可带 `auto_grade`） |
| GET/PATCH/DELETE | `/api/teacher/tasks/{id}` | 详情；PATCH 可改 `allow_early_submit`、`auto_grade`（已公开 409；改 `auto_grade` 会重判已交填空） |
| GET/PUT | `/api/teacher/tasks/{id}/roster` | 名单；PUT 仅草稿，成功 200 JSON |
| POST | `/api/teacher/tasks/{id}/split` | 草稿按班拆分子任务，成功后删原任务 |
| POST | `/api/teacher/tasks/{id}/publish` | 发布，冻结名单，写 `published_at`/`closes_at` |
| POST | `/api/teacher/tasks/{id}/close` | 收卷，提交正在作答的试卷，任务仍进行中 |
| POST | `/api/teacher/tasks/{id}/end` | 结束任务，转入历史后才能公开 |
| POST | `/api/teacher/tasks/{id}/grade` | 确认批改 |
| POST | `/api/teacher/tasks/{id}/release` | 公开试卷（须已结束且全部待手工题已确认） |
| GET | `/api/teacher/tasks/{id}/monitor` | 未进入/作答中/已提交 |
| GET | `/api/teacher/tasks/{id}/results` | 成绩列表（`code_items` 为当前待手工题） |
| GET | `/api/teacher/tasks/{id}/results.csv` | UTF-8 BOM CSV |
| GET | `/api/teacher/tasks/{id}/analysis` | 分析；可选 `?pass_score=` |
| PUT | `/api/teacher/attempts/{id}/code-scores` | 待手工题给分（编程、空参考答案，或关闭自动批改后的填空） |
| POST | `/api/teacher/attempts/{id}/confirm-marking` | 确认该生批改 |

分析接口返回平均/最高/最低/中位、及格率、分数段人数，以及 `roster_count`、`stddev`、`q1`/`q3`、优秀+/优秀/良好/及格/不及格互斥构成、逐题 `stem`/`options`/`answer`、题型 `count`/`correct_rate`、按班统计。

`median` / `q1` / `q3` 为已交卷总分的线性插值分位（位置 `(n-1)×p`）。`stddev` 为样本标准差。`cv` 为 `stddev / average`。

## 学生（需会话）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/student/paper` | 当前试卷；考试未公开时省略答案与得分 |
| PUT | `/api/student/answers` | 保存答案；已提交 409 |
| POST | `/api/student/submit` | 提交；禁止提前交卷且未到时 403 |
| GET | `/api/student/result` | 练习提交后或考试已公开后返回逐题详情（含 `options`） |

`POST /api/student/enter` 体：`client_id`，可选 `task_id`（省略则取最近 `published`/`released` 任务），`class_name` 可空，`name` 与 `student_no` 至少一项。

## 前端约定

- 讲台令牌：`localStorage exam_teacher_token`
- 学生令牌：`exam_student_token`；客户端标识 `exam_student_client_id`
- 学生页 `?t=` 绑定任务
- 开发反代：Vite `server.proxy["/api"] -> http://127.0.0.1:8080`
- `allowedHosts` 含 `.monkeycode-ai.online`

`median` / `q1` / `q3` 为已交卷总分的线性插值分位（位置 `(n-1)×p`）。`stddev` 为样本标准差。`cv` 为 `stddev / average`。
