use clap::Parser;
use mimalloc::MiMalloc;
use nix_compat::nix_daemon::handler::NixDaemon;
use nix_daemon::TvixDaemon;
use std::{error::Error, sync::Arc};
use tokio_listener::SystemOptions;
use tracing::error;
use tvix_store::utils::{construct_services, ServiceUrlsGrpc};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Run Nix-compatible store daemon backed by tvix.
#[derive(Parser)]
struct Cli {
    #[clap(flatten)]
    service_addrs: ServiceUrlsGrpc,

    /// The address to listen on. Must be a unix domain socket.
    #[clap(flatten)]
    listen_args: tokio_listener::ListenerAddressLFlag,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();

    tokio::select! {
        res = tokio::signal::ctrl_c() => {
            res?;
            Ok(())
        },
        res = run(cli) => {
            res
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (blob_service, directory_service, path_info_service, _nar_calculation_service) =
        construct_services(cli.service_addrs).await?;

    let listen_address = cli.listen_args.listen_address.unwrap_or_else(|| {
        "/tmp/tvix-daemon.sock"
            .parse()
            .expect("invalid fallback listen address")
    });

    let mut listener = tokio_listener::Listener::bind(
        &listen_address,
        &SystemOptions::default(),
        &cli.listen_args.listener_options,
    )
    .await?;

    let io = Arc::new(TvixDaemon::new(
        blob_service,
        directory_service,
        path_info_service,
    ));

    while let Ok((connection, _)) = listener.accept().await {
        let io = io.clone();
        tokio::spawn(async move {
            match NixDaemon::initialize(io.clone(), connection).await {
                Ok(mut daemon) => {
                    if let Err(error) = daemon.handle_client().await {
                        match error.kind() {
                            std::io::ErrorKind::UnexpectedEof => {
                                // client disconnected, nothing to do
                            }
                            _ => {
                                // otherwise log the error and disconnect
                                error!(error=?error, "client error");
                            }
                        }
                    }
                }
                Err(error) => {
                    error!(error=?error, "nix-daemon handshake failed");
                }
            }
        });
    }
    Ok(())
}
