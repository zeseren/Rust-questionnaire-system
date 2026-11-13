//! 真实 PostgreSQL 集成测试：受 `TEST_DATABASE_URL` 门控。
//!
//! 未设置该环境变量时直接返回，`cargo test --workspace` 在纯内存下不依赖任何外部服务。
//! 设置后会在目标库上重建 `schema.sql` 的表结构，验证标题子串筛选、`login_once`
//! 与 `screen_out` 共享的唯一约束、`question_states` 持久化以及首页计数聚合。
//!
//! 该测试会删除并重建下列表，只能指向一次性测试库：
//! `audits`、`responses`、`versions`、`surveys`、`sessions`、`users`。

use chrono::{Duration, Utc};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement, Value};
use serde_json::json;
use uuid::Uuid;

const 建表脚本: &str = include_str!("../src/schema.sql");
const 清理脚本: &str =
    "DROP TABLE IF EXISTS audits, responses, versions, surveys, sessions, users CASCADE;";

/// 未设置 `TEST_DATABASE_URL` 时返回 `None`；设置时清理并重建全部表。
async fn 测试库() -> Option<DatabaseConnection> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let db = Database::connect(url)
        .await
        .expect("TEST_DATABASE_URL 必须可以连接 PostgreSQL");
    db.execute_unprepared(清理脚本)
        .await
        .expect("清理旧测试表失败");
    db.execute_unprepared(建表脚本)
        .await
        .expect("执行 schema.sql 失败");
    Some(db)
}

fn 语句(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Postgres, sql, values)
}

async fn 执行(db: &DatabaseConnection, sql: &str, values: Vec<Value>) -> Result<(), DbErr> {
    db.execute_raw(语句(sql, values)).await.map(|_| ())
}

async fn 创建用户(db: &DatabaseConnection, 用户名: &str) -> Uuid {
    let id = Uuid::now_v7();
    执行(
        db,
        r#"INSERT INTO users (id, username, password_hash, role, disabled, created_at)
           VALUES ($1, $2, $3, 'user', false, $4)"#,
        vec![
            id.into(),
            用户名.to_owned().into(),
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$hash"
                .to_owned()
                .into(),
            Utc::now().into(),
        ],
    )
    .await
    .expect("创建测试用户失败");
    id
}

/// 插入一份问卷；非未发布状态会同时创建一个发布版本以满足 CHECK 与外键约束。
async fn 创建问卷(
    db: &DatabaseConnection,
    owner_id: Uuid,
    title: &str,
    status: &str,
    mode: Option<&str>,
    deadline: Option<chrono::DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let draft = json!({
        "logic": null,
        "title": title,
        "description": "",
        "purpose": "集成测试",
        "thank_you": "谢谢",
        "questions": [],
    });
    执行(
        db,
        r#"INSERT INTO surveys
           (id, owner_id, status, mode, deadline, taken_down, draft, revision,
            current_version_id, created_at, updated_at)
           VALUES ($1, $2, 'unpublished', NULL, NULL, false, $3, 1, NULL, $4, $4)"#,
        vec![id.into(), owner_id.into(), draft.clone().into(), now.into()],
    )
    .await
    .expect("插入问卷失败");
    if status == "unpublished" {
        return id;
    }
    let version_id = Uuid::now_v7();
    执行(
        db,
        r#"INSERT INTO versions (id, survey_id, number, document, published_at)
           VALUES ($1, $2, 1, $3, $4)"#,
        vec![version_id.into(), id.into(), draft.into(), now.into()],
    )
    .await
    .expect("插入发布版本失败");
    执行(
        db,
        r#"UPDATE surveys
           SET status = $2, mode = $3, deadline = $4, current_version_id = $5, updated_at = $6
           WHERE id = $1"#,
        vec![
            id.into(),
            status.to_owned().into(),
            mode.map(str::to_owned).into(),
            deadline.into(),
            version_id.into(),
            now.into(),
        ],
    )
    .await
    .expect("更新问卷状态失败");
    id
}

async fn 创建答卷(
    db: &DatabaseConnection,
    survey_id: Uuid,
    version_id: Uuid,
    respondent_id: Option<Uuid>,
    outcome: &str,
    question_states: serde_json::Value,
) -> Result<Uuid, DbErr> {
    let id = Uuid::now_v7();
    let scope = respondent_id.map_or_else(|| "anonymous".to_owned(), |id| id.to_string());
    let result = 执行(
        db,
        r#"INSERT INTO responses
           (id, survey_id, version_id, respondent_id, scope, key_hash, payload_hash,
            answers, outcome, question_states, submitted_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, '{}'::jsonb, $8, $9, $10)"#,
        vec![
            id.into(),
            survey_id.into(),
            version_id.into(),
            respondent_id.into(),
            scope.into(),
            Uuid::new_v4().to_string().into(),
            Uuid::new_v4().to_string().into(),
            outcome.to_owned().into(),
            question_states.into(),
            Utc::now().into(),
        ],
    )
    .await;
    result.map(|()| id)
}

async fn 读取整数(db: &DatabaseConnection, sql: &str, values: Vec<Value>) -> i64 {
    let row = db
        .query_one_raw(语句(sql, values))
        .await
        .expect("聚合查询失败")
        .expect("聚合查询必须返回一行");
    row.try_get("", "value").expect("聚合查询必须返回整数")
}

#[tokio::test]
async fn 真实_postgresql_迁移与契约() {
    let Some(db) = 测试库().await else {
        return;
    };

    // 1. 标题子串是字面匹配：`%` 不作为 LIKE 通配符。
    let 所有者 = 创建用户(&db, "owner_title").await;
    创建问卷(&db, 所有者, "春季问卷", "unpublished", None, None).await;
    创建问卷(&db, 所有者, "秋季问卷", "unpublished", None, None).await;
    let 字面标题 = 创建问卷(&db, 所有者, "100% 满意度", "unpublished", None, None).await;
    assert_ne!(字面标题, Uuid::nil());
    let 标题计数 = 读取整数(
        &db,
        r#"SELECT count(*)::bigint AS value FROM surveys
           WHERE owner_id = $1
             AND strpos(lower(draft ->> 'title'), lower($2)) > 0"#,
        vec![所有者.into(), "问卷".to_owned().into()],
    )
    .await;
    assert_eq!(标题计数, 2, "标题子串应命中两份问卷");
    let 百分号计数 = 读取整数(
        &db,
        r#"SELECT count(*)::bigint AS value FROM surveys
           WHERE owner_id = $1
             AND strpos(lower(draft ->> 'title'), lower($2)) > 0"#,
        vec![所有者.into(), "%".to_owned().into()],
    )
    .await;
    assert_eq!(百分号计数, 1, "% 必须按字面匹配");

    // 2. 首页计数：收集中的过期问卷计入 closed，总数与各状态一致。
    let 计数所有者 = 创建用户(&db, "owner_counts").await;
    let 未来 = Utc::now() + Duration::hours(1);
    let 过去 = Utc::now() - Duration::hours(1);
    创建问卷(&db, 计数所有者, "未发布", "unpublished", None, None).await;
    创建问卷(
        &db,
        计数所有者,
        "收集中",
        "collecting",
        Some("anonymous"),
        Some(未来),
    )
    .await;
    创建问卷(
        &db,
        计数所有者,
        "已过期",
        "collecting",
        Some("anonymous"),
        Some(过去),
    )
    .await;
    创建问卷(&db, 计数所有者, "已关闭", "closed", Some("anonymous"), None).await;
    创建问卷(
        &db,
        计数所有者,
        "已归档",
        "archived",
        Some("anonymous"),
        None,
    )
    .await;
    let row = db
        .query_one_raw(语句(
            r#"SELECT
                   count(*) FILTER (WHERE status = 'unpublished') AS unpublished,
                   count(*) FILTER (WHERE status = 'collecting'
                       AND (deadline IS NULL OR deadline > $2)) AS collecting,
                   count(*) FILTER (WHERE status = 'closed'
                       OR (status = 'collecting' AND deadline <= $2)) AS closed,
                   count(*) FILTER (WHERE status = 'archived') AS archived
               FROM surveys WHERE owner_id = $1"#,
            vec![计数所有者.into(), Utc::now().into()],
        ))
        .await
        .expect("计数查询失败")
        .expect("计数查询必须返回一行");
    let 计数 = |列: &str| -> i64 { row.try_get("", 列).expect("计数列必须存在") };
    assert_eq!(计数("unpublished"), 1);
    assert_eq!(计数("collecting"), 1);
    assert_eq!(计数("closed"), 2);
    assert_eq!(计数("archived"), 1);
    assert_eq!(
        计数("unpublished") + 计数("collecting") + 计数("closed") + 计数("archived"),
        5,
        "四类状态必须覆盖全部问卷"
    );

    // 3. login_once 下 complete 与 screen_out 共享 (survey_id, respondent_id) 唯一约束。
    let 发布者 = 创建用户(&db, "owner_login_once").await;
    let 问卷 = 创建问卷(
        &db,
        发布者,
        "登录限次",
        "collecting",
        Some("login_once"),
        None,
    )
    .await;
    let version_id = db
        .query_one_raw(语句(
            "SELECT current_version_id AS value FROM surveys WHERE id = $1",
            vec![问卷.into()],
        ))
        .await
        .expect("读取版本失败")
        .expect("问卷必须存在")
        .try_get::<Uuid>("", "value")
        .expect("current_version_id 必须存在");
    let 填写者 = 创建用户(&db, "respondent_login_once").await;
    创建答卷(&db, 问卷, version_id, Some(填写者), "complete", json!({}))
        .await
        .expect("首份完成答卷必须插入");
    let 冲突 = 创建答卷(&db, 问卷, version_id, Some(填写者), "screen_out", json!({}))
        .await
        .expect_err("同一账号的第二份答卷必须被唯一约束拒绝");
    assert!(
        matches!(
            冲突.sql_err(),
            Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
        ),
        "筛选结束与正常完成必须共享唯一约束：{冲突}"
    );

    // 3.1 并发提交同样只能有一份成功。
    let 并发问卷 = 创建问卷(
        &db,
        发布者,
        "并发登录限次",
        "collecting",
        Some("login_once"),
        None,
    )
    .await;
    let 并发版本 = db
        .query_one_raw(语句(
            "SELECT current_version_id AS value FROM surveys WHERE id = $1",
            vec![并发问卷.into()],
        ))
        .await
        .expect("读取版本失败")
        .expect("问卷必须存在")
        .try_get::<Uuid>("", "value")
        .expect("current_version_id 必须存在");
    let 并发填写者 = 创建用户(&db, "respondent_concurrent").await;
    let 第一份 = 创建答卷(
        &db,
        并发问卷,
        并发版本,
        Some(并发填写者),
        "complete",
        json!({}),
    );
    let 第二份 = 创建答卷(
        &db,
        并发问卷,
        并发版本,
        Some(并发填写者),
        "screen_out",
        json!({}),
    );
    let (第一份, 第二份) = tokio::join!(第一份, 第二份);
    assert_eq!(
        (第一份.is_ok() as usize) + (第二份.is_ok() as usize),
        1,
        "并发提交必须恰好一份成功：{第一份:?} / {第二份:?}"
    );

    // 4. question_states 持久化为对象，并拒绝非对象 JSONB。
    let 题目 = Uuid::now_v7();
    let mut 状态 = serde_json::Map::new();
    状态.insert(题目.to_string(), json!("skipped"));
    let 状态 = serde_json::Value::Object(状态);
    let 答卷 = 创建答卷(&db, 并发问卷, 并发版本, None, "screen_out", 状态.clone())
        .await
        .expect("匿名答卷必须可以插入");
    let 读取 = db
        .query_one_raw(语句(
            "SELECT outcome, question_states FROM responses WHERE id = $1",
            vec![答卷.into()],
        ))
        .await
        .expect("读取答卷失败")
        .expect("答卷必须存在");
    assert_eq!(
        读取
            .try_get::<String>("", "outcome")
            .expect("outcome 必须存在"),
        "screen_out"
    );
    assert_eq!(
        读取
            .try_get::<serde_json::Value>("", "question_states")
            .expect("question_states 必须存在"),
        状态,
        "三态必须按 JSON 对象原样持久化"
    );
    let 非对象 = 创建答卷(&db, 并发问卷, 并发版本, None, "complete", json!([])).await;
    assert!(
        非对象.is_err(),
        "question_states 必须是 JSON 对象，数组应被 CHECK 拒绝"
    );

    db.close().await.expect("关闭测试连接失败");
}
