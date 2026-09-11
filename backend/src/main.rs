//! 二进制入口只负责启动；具体业务放在库模块，便于独立测试。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    questionnaire_api::run().await
}
