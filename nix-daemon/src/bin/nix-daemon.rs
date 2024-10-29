use clap::Parser;
use mimalloc::MiMalloc;
use std::error::Error;
use tokio::io::AsyncWriteExt;
use tokio_listener::SystemOptions;
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

    #[cfg(feature = "otlp")]
    /// Whether to configure OTLP. Set --otlp=false to disable.
    #[arg(long, default_missing_value = "true", default_value = "true", num_args(0..=1), require_equals(true), action(clap::ArgAction::Set))]
    otlp: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();

    let tracing_handle = {
        let mut builder = tvix_tracing::TracingBuilder::default();
        builder = builder.enable_progressbar();
        #[cfg(feature = "otlp")]
        {
            if cli.otlp {
                builder = builder.enable_otlp("tvix.daemon");
            }
        }
        builder.build()?
    };

    tokio::select! {
        res = tokio::signal::ctrl_c() => {
            res?;
            if let Err(e) = tracing_handle.force_shutdown().await {
                eprintln!("failed to shutdown tracing: {e}");
            }
            Ok(())
        },
        res = run(cli) => {
            if let Err(e) = tracing_handle.shutdown().await {
                eprintln!("failed to shutdown tracing: {e}");
            }
            res
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (_blob_service, _directory_service, _path_info_service, _nar_calculation_service) =
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

    while let Ok((mut connection, _)) = listener.accept().await {
        tokio::spawn(async move {
            let ucred = connection
                .try_borrow_unix()
                .and_then(|u| u.peer_cred().ok());

            // For now we just write the connected process credentials into the connection.
            let _ = connection
                .write_all(format!("Hello {:?}", ucred).as_bytes())
                .await;
        });
    }
    Ok(())
}
