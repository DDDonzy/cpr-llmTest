use gateway_plugin_sdk::client::{PluginSession, SessionConfig};
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--print-manifest"] {
        println!(
            "{}",
            serde_json::to_string(&cpr_account_dialogue::manifest()?)?
        );
        return Ok(());
    }
    let session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig::default(),
    )
    .await?;
    if session.handshake().plugin_id != "donzy.account-dialogue"
        || session.handshake().contributes != cpr_account_dialogue::manifest()?.contributes
        || session.handshake().configuration != serde_json::json!({})
    {
        return Err("invalid handshake".into());
    }
    session.run(cpr_account_dialogue::plugin()?).await?;
    Ok(())
}
