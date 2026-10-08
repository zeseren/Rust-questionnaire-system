# 后端成熟 crate 替换评估

研究与实施日期：2026-10-08。依据后端源码、根目录 `Cargo.lock`、Context7 及上游文档，已完成 `utoipa-axum`、`cookie` 和 `pest` 三处替换，并升级后端依赖。限流、整套会话、图算法暂时保留现有实现；下面记录各项取舍及实际验证。

项目尚未正式发布，本次不增加历史格式兼容分支或数据迁移。保留某段实现的理由是当前业务规则和维护成本。已采用版本和检查结果见末尾“实施结果与验证”；其余候选版本为本次查阅记录。

**当前已经复用的部分**

[依赖声明](../backend/Cargo.toml)已经采用 Axum、Tokio、SeaORM / SQLx、Serde、Argon2、SHA-256、UUID、Chrono、URL、CSV、thiserror 和 tracing。这些基础能力无需再找替代实现。

尤其需要区分“自己实现通用算法”和“调用 crate 后补充业务规则”：`identity.rs` 已调用 `uuid`，再限制规范身份；`reports.rs` 已调用 `csv`，再规定导出列及公式防护；`error.rs` 已使用 `thiserror`，再实现 HTTP 错误契约。这些包装并非重复实现库。

| 当前部分 | 候选 crate | 建议 | 主要收益与代价 |
| --- | --- | --- | --- |
| 路由与 OpenAPI 分开登记 | `utoipa-axum` | 已实施 | 同一份 handler 注解生成路由和文档，减少接口增删时的遗漏 |
| Cookie 字符串解析与拼接 | `cookie` | 已实施 | 使用标准解析器和构造器；保留重复 session Cookie 拒绝策略 |
| DSL 文本的词法、语法与优先级解析 | `pest` + `pest_derive` | 已实施 | 文法集中、解析位置可追踪；绑定、AST 构造、资源限制仍由项目负责 |
| 只替换 DSL 分词 | `logos` | 保留为备选 | 接管词法扫描，保留原优先级解析器；收益局限于分词部分 |
| 单实例请求限流 | `governor` | 有条件 | 接管限流状态与算法，但 GCRA 改变固定窗口行为 |
| 变量拓扑排序、流程支配关系 | `petgraph` | 暂缓 | 图算法现成；当前规模小，还需自行保持确定性和业务校验 |
| 环境配置、CLI、局部更新三态 | `envy`、`clap`、`serde_with` | 暂缓 | 都能替换，但当前分别只有少量字段、三个子命令和一个三态字段 |
| 整套登录与会话 | `tower-sessions` + `axum-login` | 当前保留 | 可提供通用认证框架；现有事务内重验、撤销与哈希存储仍需定制 |
| 问卷规则、数值规范化、流程执行 | 通用校验或表达式库 | 当前保留 | 主要复杂度来自领域语义，换库后仍需实现相同规则 |

**1. 路由与 OpenAPI：最明确的替换收益**

位置：[auth.rs](../backend/src/auth.rs) 及其他四个业务模块的 `router`，以及 [lib.rs](../backend/src/lib.rs) 的 `router`。

当前共有 31 个 `#[utoipa::path]` 操作声明。改造前，添加接口要同步修改 `.route(...)`、handler 路径与方法注解、`#[openapi(paths(...))]` 列表。现在由 `OpenApiRouter` 与 `routes!` 统一登记，通过 `split_for_parts()` 取得 Axum Router 和 OpenAPI 文档；同路径的多个方法在一个 `routes!` 中合并。

模块入口已改为 `router() -> OpenApiRouter<AppState>`，删除独立 `ApiDoc` 列表；响应状态、中文描述、鉴权方案保留在 handler 注解中。状态、提取器和业务函数继续复用，文档只在路由组装时生成。治理列表使用独立的 `AdminSurveyPage` 名称，解决原先与发布者 `SurveyPage` 的组件冲突。

版本必须配对：

| 实际查阅的版本 | 上游声明的依赖 | 对当前项目的结论 |
| --- | --- | --- |
| `utoipa-axum 0.2.0` | `axum 0.8.0`、`utoipa 5.0.0`；MSRV 1.75 | 可配套升级前的 Axum 0.8.9、utoipa 5.5.0 |
| `utoipa-axum 0.3.0` | `axum 0.8.4`、`utoipa 6.0.0`；MSRV 1.88 | 本轮采用，与 utoipa 6.0.0 一同升级；实际仍生成 OpenAPI 3.1.0 |

来源：[0.2.0 使用说明](https://docs.rs/utoipa-axum/0.2.0/utoipa_axum/)、[0.2.0 依赖声明](https://docs.rs/crate/utoipa-axum/0.2.0/source/Cargo.toml)、[0.3.0 依赖声明](https://docs.rs/crate/utoipa-axum/0.3.0/source/Cargo.toml)。这是 utoipa 官方仓库内的集成库，可直接延续现有技术栈。

**2. Cookie：优先使用底层 cookie crate**

位置：[auth.rs](../backend/src/auth.rs) 的 `session_token` 与 `session_cookie`。改造前分别手工拆分请求头、拼接 `Set-Cookie`；现在使用 `Cookie::split_parse` 和 `Cookie::build`。

直接使用 `cookie 0.18.2` 接管 HTTP Cookie 的通用处理。当前只需要一个 session Cookie，会话存储与撤销继续使用原业务事务。

实现保留以下策略：

- 遍历全部 `Cookie` 请求头；出现重复的 `session` 时拒绝认证。
- session 值必须满足当前 64 位十六进制字符串规则；使用不做百分号解码的解析入口。
- 设置 `HttpOnly`、`SameSite=Lax`、`Path=/`、`Max-Age`，按配置决定 `Secure`；退出时明确生成清除 Cookie。
- `value_raw` 借用原请求头中的值，随后检查十六进制格式；无关非法片段可跳过，非法 session 值拒绝认证。

`CookieJar::add_original` 会覆盖已存在的同名 Cookie。因此直接改成 `axum-extra::CookieJar` 后再 `get("session")`，会丢失检测重复项的机会。应在 Cookie 集合折叠前完成此项检查。

这是一项局部维护改进，净减少代码量有限。来源：[Cookie 解析与构造](https://docs.rs/cookie/0.18.2/cookie/struct.Cookie.html)、[CookieJar 同名覆盖行为](https://docs.rs/cookie/0.18.2/cookie/struct.CookieJar.html)。

**3. DSL 文本解析：通用解析机制可以换，语义编译继续复用**

改造前 [logic_text.rs](../backend/src/logic_text.rs) 共 607 行，包含分词、优先级、声明解析、别名绑定和语义错误定位。现在将词法和完整文法集中到 [logic_text.pest](../backend/src/logic_text.pest)，用 pest 匹配解析树、`PrattParser` 处理优先级；Rust 适配层负责身份、AST 与预算，不能把整份文件都计为可删除代码。

| 方案 | 可替换范围 | 适用判断 |
| --- | --- | --- |
| `pest 2.9.2` + `pest_derive` | PEG 文法、语法匹配、源位置；`PrattParser` 处理运算优先级 | 已采用，完整文法可以集中审阅 |
| `logos 0.16.1` | 标识符、数字、字符串、运算符、注释扫描 | 适合只替换分词、保留原语法解析器的范围 |
| `winnow 1.0.4` | 用 Rust 解析组合子逐步替换扫描与语法辅助函数 | 偏好文法仍写在 Rust 中时可选；现有锁文件已有间接依赖，但仍需声明直接依赖 |

三者是不同粒度的方案。本轮选择完整的 pest 文法，没有另外引入 logos 或 winnow 作为直接依赖。

`pest` 上游提供文法书、Pratt 解析器和源位置 API，并列出 Tera、Handlebars 等生态使用者；`logos` 有独立手册和派生式词法定义；`winnow` 提供组合子文档和版本变更记录。这里选择的是通用解析能力，不依赖某个问卷专用小众包。

替换时的主要约束来自当前契约：

- 数字 token 保留原文；不能先转 `f64`，否则会丢失精度检查与字节预算所需信息。
- 字符串继续使用 JSON 字符串规则，含转义与 Unicode 标量检查。DSL 词法空白当前为 ASCII 空格、制表和换行；答案文本的 Unicode `White_Space` 规范是另一层规则。
- 同级运算左结合；`!` 是表达式前缀，负号只用于数字常量。不能直接套用计算器示例中的任意一元负号。
- 保留 1 MiB 源文本、65,536 token、64 字符别名、解析深度，以及 AST 节点数、深度和总容量限制。
- AST 构造过程中继续检查预算，不能等超深 AST 构造完后才验证。`pest::set_call_limit` 统计的是解析调用次数，不能代替这些领域预算。
- 保留错误分类、AST 路径和 Unicode 标量列号。库提供字节 span 后，仍需映射成当前诊断。
- 保留身份绑定与 `DocumentLogic::compile` 调用；Rust 与 TypeScript 继续通过现有共享文本和语义样例。

来源：[pest 使用说明](https://docs.rs/crate/pest/2.9.2)、[PrattParser](https://pest.rs/book/precedence.html)、[解析调用限制](https://docs.rs/pest/2.9.2/pest/fn.set_call_limit.html)、[logos](https://docs.rs/logos/0.16.1/logos/)、[winnow](https://docs.rs/winnow/1.0.4/winnow/)。

**4. 限流：governor 可用，但属于行为调整**

位置：[security.rs](../backend/src/security.rs) 的 `Limits`、`Window` 与 `admit`（43 行起）。现有实现为整个实例共享的两个固定窗口，只在匹配的写请求中计数。

`governor 0.10.4` 使用 GCRA，可以接管限流算法和并发状态。较小的集成方式是在 `Limits` 中放两个 direct limiter，继续由 `guard` 选择请求范围并将拒绝转换为现有的 `429 rate_limited`。

固定窗口在窗口到期时重置全部额度；GCRA 根据经过时间逐步恢复容量。即使都配置成“每分钟 N 次”，突发和等待行为也不同。因此只有准备调整限流策略时才建议换；目前约二十行的 `admit` 没有形成明显维护负担。

如果选 `tower_governor 0.8.0`，应使用 `GlobalKeyExtractor` 并保持两个独立限额。它默认按客户端 IP 区分，不符合当前“不收集 IP、按实例限流”的约定。接入还需保留 CSRF 检查顺序、JSON 错误及拒绝响应上的安全头。两种内存方案都不自动提供跨实例共享限流。

来源：[governor 算法](https://docs.rs/crate/governor/0.10.4)、[tower_governor](https://docs.rs/crate/tower_governor/0.8.0)、[全局限流提取器](https://docs.rs/tower_governor/0.8.0/tower_governor/key_extractor/struct.GlobalKeyExtractor.html)。

**5. 可以替换，但当前不划算的部分**

| 位置与候选 | 暂缓原因 |
| --- | --- |
| [config.rs](../backend/src/config.rs)：`envy` 或 `figment` | 共 7 个配置字段，很多代码在校验 PostgreSQL、规范 origin、HTTPS / Cookie 联动和数值范围。库只省掉取值与反序列化，仍需保留这些校验及中文错误；目前也没有多来源配置需求 |
| [lib.rs](../backend/src/lib.rs)：`clap` | 只有 `serve`、`migrate`、`create-admin` 三个子命令。出现参数、帮助、补全需求后收益才明显；管理员秘密仍应通过环境变量输入 |
| [surveys.rs](../backend/src/surveys.rs) 的 `deserialize_deadline`：`serde_with::rust::double_option` | 缺省、null、时间值的三态语义完全匹配，但目前只替换一个很短的函数。后续有多处同类字段时再统一更划算 |
| [http.rs](../backend/src/http.rs) 的 `optional_u64`：通用 Serde 字符串适配器 | 当前明确拒绝 `+1`，只接受 ASCII 十进制数字；只调用 `FromStr` 的适配器不能直接代替这一限制 |
| [security.rs](../backend/src/security.rs)：`tower-http::SetResponseHeaderLayer` | 能接管两个响应头；目前只省十余行。未来已有 tower-http 需求时可以顺便统一，使用覆盖模式并让拒绝响应也经过该层 |
| [logic_document.rs](../backend/src/logic_document.rs) 的变量排序、[flow.rs](../backend/src/flow.rs) 的图分析：`petgraph` | 当前最多 100 个变量、256 个流程节点。变量排序还要求每次优先选择 UUID 最小的可执行变量，通用 `toposort` 不承诺这一选择规则；流程还需前向限制、题目归属与缺失读取分析。库提供图算法后仍要保留这些处理 |
| [domain.rs](../backend/src/domain.rs) 等字段校验：`garde` | 能表达字段与自定义校验，但题型关联、全局身份唯一、动态规则和提交预算仍需项目代码。还要转换成带 `question_id` 的现有错误，当前净收益不大 |

来源：[envy](https://docs.rs/envy/latest/envy/)、[figment 环境配置](https://docs.rs/figment/latest/figment/providers/struct.Env.html)、[clap](https://docs.rs/clap/latest/clap/)、[double_option](https://docs.rs/serde_with/latest/serde_with/rust/double_option/index.html)、[响应头中间件](https://docs.rs/tower-http/latest/tower_http/set_header/response/struct.SetResponseHeaderLayer.html)、[petgraph 拓扑排序](https://docs.rs/petgraph/0.8.3/petgraph/algo/fn.toposort.html)、[garde](https://docs.rs/garde/latest/garde/)。

**6. 当前应保留的领域与事务实现**

`tower-sessions` 与 `axum-login` 是值得了解的通用会话、认证方案，但当前 [auth.rs](../backend/src/auth.rs) 和 [repository.rs](../backend/src/repository.rs) 还负责数据库仅存 token 摘要、登录替换旧会话、改密或禁用撤销会话，以及持有用户行锁后在同一事务内重验原会话。这些约束不会由通用中间件自动完成。

`axum-login` 的 `session_auth_hash` 能帮助实现改密失效；它仍不等于当前写事务中的认证重验。切换框架需要设计自定义存储和业务接入，不能按“删除 auth.rs”估算收益。以后需要外部认证源、复杂权限或大量会话数据时再评估更合适。来源：[tower-sessions](https://docs.rs/tower-sessions/latest/tower_sessions/)、[axum-login](https://docs.rs/axum-login/latest/axum_login/)。

[logic.rs](../backend/src/logic.rs)、[logic_document.rs](../backend/src/logic_document.rs)、[flow.rs](../backend/src/flow.rs) 的复杂度主要来自题目类型绑定、missing / unknown、短路、合流处可用性、改答清理与最终路径重建。通用表达式或脚本引擎即使可扩展，也仍需实现这些适配；目前不建议整体替换。

[domain.rs](../backend/src/domain.rs) 的 `finite_number` / `canonical_number` 以有限 `f64` 和两端一致规则为基础，额外保留 JSON 原文用于精度与传输预算检查。改用十进制数类型会改变数值模型，应作为独立需求处理。

[reports.rs](../backend/src/reports.rs) 的 CSV 编码已经由 `csv` 完成。`formula_safe`、答案状态列、适用人数和稳定均值属于导出及统计规则，普通 CSV 或统计库不会自动补齐。

**7. 实施结果与验证**

已更新 [backend/Cargo.toml](../backend/Cargo.toml) 和根锁文件，并执行 `cargo update`、`cargo fetch --locked`。主要版本如下，其余兼容范围内的直接及传递依赖一并由 Cargo 解析更新。

| 依赖 | 升级前 | 当前锁定 |
| --- | --- | --- |
| utoipa / utoipa-axum | 5.5.0 / 未引入 | 6.0.0 / 0.3.0 |
| cookie | 未引入 | 0.18.2 |
| pest / pest_derive | 未引入 | 2.9.2 / 2.9.2 |
| argon2 | 0.5.3 | 0.6.0 |
| sha2（直接依赖） | 0.10.9 | 0.11.0 |
| SeaORM / sea-orm-migration | 2.0.2 / 2.0.2 | 2.0.4 / 2.0.4 |
| Tokio | 1.53.1 | 1.53.2 |
| getrandom（直接依赖） | 未直接引入 | 0.4.3 |

Argon2 0.6 的 `hash_password` 自动生成随机盐，仍在 `spawn_blocking` 中执行，返回 PHC 字符串；错误密码按新错误枚举转换成原有认证失败。Session 随机值改为 `getrandom::fill`。Axum 0.8.9、SQLx 0.9.0 和 `serde_json/arbitrary_precision` 保留。SQLx 的传递依赖仍包含 sha2 0.10.9，由锁文件分别解析。来源：[Argon2 0.6](https://docs.rs/argon2/0.6.0/argon2/)、[PasswordHasher](https://docs.rs/password-hash/0.6.1/password_hash/trait.PasswordHasher.html)、[utoipa 6 变更记录](https://docs.rs/crate/utoipa/6.0.0/source/CHANGELOG.md)。

验证结果：

- stable + LLVM：工作区测试 **34/34**（domain 20、logic 5、flow 5、logic_text 4），`cargo fmt --all -- --check`、clippy `--all-targets -- -D warnings` 和开发构建通过。
- 文本共享案例从 17 条增加到 **23 条**，补充负号和注释、科学记数法原文、布尔名称引用、Unicode 字符与错误列、关键字边界；Rust 另核对 63 层括号边界、混合前缀/函数超限、负号范围与函数参数类型。前端 `pnpm run test:regression` **19/19** 通过。
- 真实 PostgreSQL 18.1：对照旧版文档，**28 个路径、31 个方法、查询参数、安全方案和响应状态码集合一致**，所有组件引用可解析；发布者和管理员列表分别指向正确 Schema。31 个操作均经实际 HTTP 成功执行。
- 共 64 次 HTTP 检查涵盖 Cookie 解析、重复头拒绝、会话轮换、改密/禁用撤销、清除及 Secure 属性；核对数据库密码为 Argon2id PHC、Session 只存 SHA-256 摘要；六题型问卷保存、发布重放、幂等提交、详情/统计/CSV、生命周期及管理员审计通过。403、404、405、400、413 的 JSON 错误与 no-store/nosniff 同时核对。
- `cargo metadata --locked --offline` 中没有声明高于 Rust 1.94 的依赖 MSRV；本轮实际编译使用 stable 1.99.0，未额外安装 1.94 工具链验证。

临时 HTTP 核对程序、日志和独立数据库均位于忽略的 `target/`，没有新增项目测试框架、生产脚本或数据库迁移。本轮未重做浏览器、权限并发、压测、release 或 Cranelift 验证，不从此次替换推断性能收益。
