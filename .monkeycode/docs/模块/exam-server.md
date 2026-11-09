# exam-server

Axum 服务。入口 `src/main.rs`，路由 `src/lib.rs`。

| 文件 | 职责 |
|---|---|
| `auth.rs` | 教师设口令、登录、改口令、中间件 |
| `lan.rs` | 局域网 IPv4 |
| `banks.rs` | 题库与题目 CRUD、HTML 导入导出 |
| `students.rs` | 班级、学生、CSV、任务名单 |
| `papers.rs` | 试卷模板、预览、调分 |
| `paper.rs` | 快照生成、`student_items`、任务行、`needs_manual` / `grade_blob` |
| `tasks.rs` | 任务 CRUD、拆班、发布/收卷/结束/批改/公开、监控 |
| `student.rs` | 学生进入、试卷、保存、提交、结果 |
| `results.rs` | 成绩、CSV、分析、待手工题给分 |
| `db.rs` | schema 与列迁移（含 `exam_tasks.auto_grade`、`exam_papers.auto_grade`） |
| `config.rs` | `EXAM_DATA_DIR` / `EXAM_BIND` |
| `error.rs` | 统一错误体 |

测试：`crates/exam-server/tests/`（auth、banks、students、exam_flow、store）。
