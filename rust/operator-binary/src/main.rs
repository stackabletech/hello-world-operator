mod affinity;
mod controller;
mod crd;
mod operations;
mod product_logging;

use crate::controller::HELLO_FULL_CONTROLLER_NAME;

use clap::{crate_description, crate_version, Parser};
use crd::{HelloCluster, APP_NAME};
use futures::stream::StreamExt;
use stackable_operator::{
    cli::{Command, ProductOperatorRun},
    k8s_openapi::api::{
        apps::v1::StatefulSet,
        core::v1::{ConfigMap, Service},
    },
    kube::{
        core::DeserializeGuard,
        runtime::{
            events::{Recorder, Reporter},
            watcher, Controller,
        },
    },
    logging::controller::report_controller_reconciled,
    CustomResourceExt,
};
use std::sync::Arc;

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

const OPERATOR_NAME: &str = "hello.stackable.tech";

#[derive(Parser)]
#[clap(about, author)]
struct Opts {
    #[clap(subcommand)]
    cmd: Command,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    match opts.cmd {
        Command::Crd => HelloCluster::print_yaml_schema(built_info::PKG_VERSION)?,
        Command::Run(ProductOperatorRun {
            product_config,
            watch_namespace,
            tracing_target,
            cluster_info_opts,
        }) => {
            stackable_operator::logging::initialize_logging(
                "HELLO_OPERATOR_LOG",
                APP_NAME,
                tracing_target,
            );
            stackable_operator::utils::print_startup_string(
                crate_description!(),
                crate_version!(),
                built_info::GIT_VERSION,
                built_info::TARGET,
                built_info::BUILT_TIME_UTC,
                built_info::RUSTC_VERSION,
            );

            let product_config = product_config.load(&[
                "deploy/config-spec/properties.yaml",
                "/etc/stackable/hello-world-operator/config-spec/properties.yaml",
            ])?;

            let client = stackable_operator::client::initialize_operator(
                Some(OPERATOR_NAME.to_string()),
                &cluster_info_opts,
            )
            .await?;

            let event_recorder = Arc::new(Recorder::new(
                client.as_kube_client(),
                Reporter {
                    controller: HELLO_FULL_CONTROLLER_NAME.to_string(),
                    instance: None,
                },
            ));

            Controller::new(
                watch_namespace.get_api::<DeserializeGuard<HelloCluster>>(&client),
                watcher::Config::default(),
            )
            .owns(
                watch_namespace.get_api::<DeserializeGuard<Service>>(&client),
                watcher::Config::default(),
            )
            .owns(
                watch_namespace.get_api::<DeserializeGuard<StatefulSet>>(&client),
                watcher::Config::default(),
            )
            .owns(
                watch_namespace.get_api::<DeserializeGuard<ConfigMap>>(&client),
                watcher::Config::default(),
            )
            .shutdown_on_signal()
            .run(
                controller::reconcile_hello,
                controller::error_policy,
                Arc::new(controller::Ctx {
                    client: client.clone(),
                    product_config,
                }),
            )
            // We can let the reporting happen in the background
            .for_each_concurrent(
                16, // concurrency limit
                |result| {
                    // The event_recorder needs to be shared across all invocations, so that
                    // events are correctly aggregated
                    let event_recorder = event_recorder.clone();
                    async move {
                        report_controller_reconciled(
                            &event_recorder,
                            HELLO_FULL_CONTROLLER_NAME,
                            &result,
                        )
                        .await;
                    }
                },
            )
            .await;
        }
    }

    Ok(())
}
