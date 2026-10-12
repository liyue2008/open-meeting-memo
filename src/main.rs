use clap::{error::ErrorKind, Parser};
use omm::{
    cli::{Cli, Response},
    core::contracts::OmmError,
};

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(error) => {
            if std::env::args_os().any(|arg| arg == "--json") {
                let response = Response {
                    schema_version: 1,
                    ok: false,
                    data: None,
                    error: Some(OmmError::invalid(
                        "invalid CLI arguments; use --help for supported commands",
                    )),
                    warnings: vec![],
                };
                println!(
                    "{}",
                    serde_json::to_string(&response).expect("serializable response")
                );
                std::process::exit(2);
            }
            error.exit();
        }
    };
    std::process::exit(omm::cli::run(cli).await);
}
