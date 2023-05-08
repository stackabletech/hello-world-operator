//! Ensures that `Pod`s are configured and running for each [`HelloCluster`]
use crate::config::{generate_index_html, generate_nginx_conf};
use crate::product_logging::{extend_role_group_config_map, resolve_vector_aggregator_address};
use crate::OPERATOR_NAME;

use crate::crd::{
    Container, HelloCluster, HelloClusterStatus, HelloRole, ServerConfig, APP_NAME, HELLO_COLOR,
    HELLO_RECIPIENT, HTTP_PORT, HTTP_PORT_NAME, INDEX_HTML, NGINX_CONF, STACKABLE_CONFIG_DIR,
    STACKABLE_CONFIG_DIR_NAME, STACKABLE_CONFIG_MOUNT_DIR, STACKABLE_CONFIG_MOUNT_DIR_NAME,
    STACKABLE_LOG_CONFIG_MOUNT_DIR, STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME, STACKABLE_LOG_DIR,
    STACKABLE_LOG_DIR_NAME,
};
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    builder::{ConfigMapBuilder, ContainerBuilder, ObjectMetaBuilder, PodBuilder},
    cluster_resources::{ClusterResourceApplyStrategy, ClusterResources},
    commons::{product_image_selection::ResolvedProductImage, rbac::build_rbac_resources},
    k8s_openapi::{
        api::{
            apps::v1::{StatefulSet, StatefulSetSpec},
            core::v1::{
                ConfigMap, ConfigMapVolumeSource, EmptyDirVolumeSource, Probe, Service,
                ServicePort, ServiceSpec, TCPSocketAction, Volume,
            },
        },
        apimachinery::pkg::{
            api::resource::Quantity, apis::meta::v1::LabelSelector, util::intstr::IntOrString,
        },
    },
    kube::{runtime::controller::Action, Resource, ResourceExt},
    labels::{role_group_selector_labels, role_selector_labels, ObjectLabels},
    logging::controller::ReconcilerError,
    product_config::{types::PropertyNameKind, ProductConfigManager},
    product_config_utils::{transform_all_roles_to_config, validate_all_roles_and_groups_config},
    product_logging::{
        self,
        spec::{
            ConfigMapLogConfig, ContainerLogConfig, ContainerLogConfigChoice,
            CustomContainerLogConfig,
        },
    },
    role_utils::RoleGroupRef,
    status::condition::{
        compute_conditions, operations::ClusterOperationsConditionBuilder,
        statefulset::StatefulSetConditionBuilder,
    },
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use strum::EnumDiscriminants;
use tracing::warn;

pub const HELLO_CONTROLLER_NAME: &str = "hellocluster";
const DOCKER_IMAGE_BASE_NAME: &str = "hello";

pub const MAX_HIVE_LOG_FILES_SIZE_IN_MIB: u32 = 10;

const OVERFLOW_BUFFER_ON_LOG_VOLUME_IN_MIB: u32 = 1;
const LOG_VOLUME_SIZE_IN_MIB: u32 =
    MAX_HIVE_LOG_FILES_SIZE_IN_MIB + OVERFLOW_BUFFER_ON_LOG_VOLUME_IN_MIB;

pub struct Ctx {
    pub client: stackable_operator::client::Client,
    pub product_config: ProductConfigManager,
}

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(strum::IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("object defines no namespace"))]
    ObjectHasNoNamespace,
    #[snafu(display("object defines no metastore role"))]
    NoServerRole,
    #[snafu(display("failed to calculate global service name"))]
    GlobalServiceNameNotFound,
    #[snafu(display("failed to calculate service name for role {rolegroup}"))]
    RoleGroupServiceNameNotFound {
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to apply global Service"))]
    ApplyRoleService {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to apply Service for {rolegroup}"))]
    ApplyRoleGroupService {
        source: stackable_operator::error::Error,
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to build ConfigMap for {rolegroup}"))]
    BuildRoleGroupConfig {
        source: stackable_operator::error::Error,
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to apply ConfigMap for {rolegroup}"))]
    ApplyRoleGroupConfig {
        source: stackable_operator::error::Error,
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to apply StatefulSet for {rolegroup}"))]
    ApplyRoleGroupStatefulSet {
        source: stackable_operator::error::Error,
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to generate product config"))]
    GenerateProductConfig {
        source: stackable_operator::product_config_utils::ConfigError,
    },
    #[snafu(display("invalid product config"))]
    InvalidProductConfig {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("object is missing metadata to build owner reference"))]
    ObjectMissingMetadataForOwnerRef {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to apply discovery ConfigMap"))]
    ApplyDiscoveryConfig {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to update status"))]
    ApplyStatus {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to parse db type {db_type}"))]
    InvalidDbType {
        source: strum::ParseError,
        db_type: String,
    },
    #[snafu(display("failed to resolve S3 connection"))]
    ResolveS3Connection {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to resolve and merge resource config for role and role group"))]
    FailedToResolveResourceConfig { source: crate::crd::Error },
    #[snafu(display("failed to create hello container [{name}]"))]
    FailedToCreateHelloContainer {
        source: stackable_operator::error::Error,
        name: String,
    },
    #[snafu(display("failed to create cluster resources"))]
    CreateClusterResources {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to delete orphaned resources"))]
    DeleteOrphanedResources {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to resolve the Vector aggregator address"))]
    ResolveVectorAggregatorAddress {
        source: crate::product_logging::Error,
    },
    #[snafu(display("failed to add the logging configuration to the ConfigMap [{cm_name}]"))]
    InvalidLoggingConfig {
        source: crate::product_logging::Error,
        cm_name: String,
    },
    #[snafu(display("failed to patch service account"))]
    ApplyServiceAccount {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to patch role binding"))]
    ApplyRoleBinding {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to build RBAC resources"))]
    BuildRbacResources {
        source: stackable_operator::error::Error,
    },
}
type Result<T, E = Error> = std::result::Result<T, E>;

impl ReconcilerError for Error {
    fn category(&self) -> &'static str {
        ErrorDiscriminants::from(self).into()
    }
}

pub async fn reconcile_hello(hello: Arc<HelloCluster>, ctx: Arc<Ctx>) -> Result<Action> {
    tracing::info!("Starting reconcile");
    let client = &ctx.client;
    let resolved_product_image: ResolvedProductImage =
        hello.spec.image.resolve(DOCKER_IMAGE_BASE_NAME);

    let validated_config = validate_all_roles_and_groups_config(
        &resolved_product_image.product_version,
        &transform_all_roles_to_config(
            hello.as_ref(),
            [(
                HelloRole::Server.to_string(),
                (
                    vec![
                        PropertyNameKind::Env,
                        PropertyNameKind::Cli,
                        PropertyNameKind::File(INDEX_HTML.to_string()),
                    ],
                    hello.spec.servers.clone().context(NoServerRoleSnafu)?,
                ),
            )]
            .into(),
        )
        .context(GenerateProductConfigSnafu)?,
        &ctx.product_config,
        false,
        false,
    )
    .context(InvalidProductConfigSnafu)?;

    let server_config = validated_config
        .get(&HelloRole::Server.to_string())
        .map(Cow::Borrowed)
        .unwrap_or_default();

    let mut cluster_resources = ClusterResources::new(
        APP_NAME,
        OPERATOR_NAME,
        HELLO_CONTROLLER_NAME,
        &hello.object_ref(&()),
        ClusterResourceApplyStrategy::from(&hello.spec.cluster_operation),
    )
    .context(CreateClusterResourcesSnafu)?;

    let (rbac_sa, rbac_rolebinding) = build_rbac_resources(
        hello.as_ref(),
        APP_NAME,
        cluster_resources.get_required_labels(),
    )
    .context(BuildRbacResourcesSnafu)?;

    let rbac_sa = cluster_resources
        .add(client, rbac_sa)
        .await
        .context(ApplyServiceAccountSnafu)?;
    cluster_resources
        .add(client, rbac_rolebinding)
        .await
        .context(ApplyRoleBindingSnafu)?;

    let server_role_service = build_server_role_service(&hello, &resolved_product_image)?;

    // we have to get the assigned ports
    cluster_resources
        .add(client, server_role_service)
        .await
        .context(ApplyRoleServiceSnafu)?;

    let vector_aggregator_address = resolve_vector_aggregator_address(&hello, client)
        .await
        .context(ResolveVectorAggregatorAddressSnafu)?;

    let mut ss_cond_builder = StatefulSetConditionBuilder::default();

    for (rolegroup_name, rolegroup_config) in server_config.iter() {
        let rolegroup = hello.server_rolegroup_ref(rolegroup_name);

        let config = hello
            .merged_config(&HelloRole::Server, &rolegroup.role_group)
            .context(FailedToResolveResourceConfigSnafu)?;

        let rg_service = build_rolegroup_service(&hello, &resolved_product_image, &rolegroup)?;
        let rg_configmap = build_server_rolegroup_config_map(
            &hello,
            &resolved_product_image,
            &rolegroup,
            rolegroup_config,
            &config,
            vector_aggregator_address.as_deref(),
        )?;
        let rg_statefulset = build_server_rolegroup_statefulset(
            &hello,
            &resolved_product_image,
            &rolegroup,
            rolegroup_config,
            &config,
            &rbac_sa.name_any(),
        )?;

        cluster_resources
            .add(client, rg_service)
            .await
            .context(ApplyRoleGroupServiceSnafu {
                rolegroup: rolegroup.clone(),
            })?;

        cluster_resources
            .add(client, rg_configmap)
            .await
            .context(ApplyRoleGroupConfigSnafu {
                rolegroup: rolegroup.clone(),
            })?;

        ss_cond_builder.add(
            cluster_resources
                .add(client, rg_statefulset)
                .await
                .context(ApplyRoleGroupStatefulSetSnafu {
                    rolegroup: rolegroup.clone(),
                })?,
        );
    }

    let cluster_operation_cond_builder =
        ClusterOperationsConditionBuilder::new(&hello.spec.cluster_operation);

    let status = HelloClusterStatus {
        conditions: compute_conditions(
            hello.as_ref(),
            &[&ss_cond_builder, &cluster_operation_cond_builder],
        ),
    };

    client
        .apply_patch_status(OPERATOR_NAME, &*hello, &status)
        .await
        .context(ApplyStatusSnafu)?;

    cluster_resources
        .delete_orphaned_resources(client)
        .await
        .context(DeleteOrphanedResourcesSnafu)?;

    Ok(Action::await_change())
}

pub fn build_server_role_service(
    hello: &HelloCluster,
    resolved_product_image: &ResolvedProductImage,
) -> Result<Service> {
    let role_name = HelloRole::Server.to_string();

    let role_svc_name = hello
        .server_role_service_name()
        .context(GlobalServiceNameNotFoundSnafu)?;
    Ok(Service {
        metadata: ObjectMetaBuilder::new()
            .name_and_namespace(hello)
            .name(role_svc_name)
            .ownerreference_from_resource(hello, None, Some(true))
            .context(ObjectMissingMetadataForOwnerRefSnafu)?
            .with_recommended_labels(build_recommended_labels(
                hello,
                &resolved_product_image.app_version_label,
                &role_name,
                "global",
            ))
            .build(),
        spec: Some(ServiceSpec {
            type_: Some(hello.spec.cluster_config.listener_class.k8s_service_type()),
            ports: Some(service_ports()),
            selector: Some(role_selector_labels(hello, APP_NAME, &role_name)),
            ..ServiceSpec::default()
        }),
        status: None,
    })
}

/// The rolegroup [`ConfigMap`] configures the rolegroup based on the configuration given by the administrator
fn build_server_rolegroup_config_map(
    hello: &HelloCluster,
    resolved_product_image: &ResolvedProductImage,
    rolegroup: &RoleGroupRef<HelloCluster>,
    role_group_config: &HashMap<PropertyNameKind, BTreeMap<String, String>>,
    merged_config: &ServerConfig,
    vector_aggregator_address: Option<&str>,
) -> Result<ConfigMap> {
    let mut hello_index_html = String::new();

    for (property_name_kind, config) in role_group_config {
        match property_name_kind {
            PropertyNameKind::File(file_name) if file_name == INDEX_HTML => {
                let recipient = config
                    .get(HELLO_RECIPIENT)
                    .map(|x| x.as_str())
                    .unwrap_or("World");
                let color = config
                    .get(HELLO_COLOR)
                    .map(|x| x.as_str())
                    .unwrap_or("#000000");

                // TODO maybe don't set these defaults in here ^

                hello_index_html = generate_index_html(recipient, color);
            }
            _ => {}
        }
    }

    let mut cm_builder = ConfigMapBuilder::new();

    cm_builder
        .metadata(
            ObjectMetaBuilder::new()
                .name_and_namespace(hello)
                .name(rolegroup.object_name())
                .ownerreference_from_resource(hello, None, Some(true))
                .context(ObjectMissingMetadataForOwnerRefSnafu)?
                .with_recommended_labels(build_recommended_labels(
                    hello,
                    &resolved_product_image.app_version_label,
                    &rolegroup.role,
                    &rolegroup.role_group,
                ))
                .build(),
        )
        .add_data(INDEX_HTML, hello_index_html)
        .add_data(NGINX_CONF, generate_nginx_conf());

    extend_role_group_config_map(
        rolegroup,
        vector_aggregator_address,
        &merged_config.logging,
        &mut cm_builder,
    )
    .context(InvalidLoggingConfigSnafu {
        cm_name: rolegroup.object_name(),
    })?;

    cm_builder
        .build()
        .with_context(|_| BuildRoleGroupConfigSnafu {
            rolegroup: rolegroup.clone(),
        })
}

/// The rolegroup [`Service`] is a headless service that allows direct access to the instances of a certain rolegroup
///
/// This is mostly useful for internal communication between peers, or for clients that perform client-side load balancing.
fn build_rolegroup_service(
    hello: &HelloCluster,
    resolved_product_image: &ResolvedProductImage,
    rolegroup: &RoleGroupRef<HelloCluster>,
) -> Result<Service> {
    Ok(Service {
        metadata: ObjectMetaBuilder::new()
            .name_and_namespace(hello)
            .name(&rolegroup.object_name())
            .ownerreference_from_resource(hello, None, Some(true))
            .context(ObjectMissingMetadataForOwnerRefSnafu)?
            .with_recommended_labels(build_recommended_labels(
                hello,
                &resolved_product_image.app_version_label,
                &rolegroup.role,
                &rolegroup.role_group,
            ))
            .build(),
        spec: Some(ServiceSpec {
            // Internal communication does not need to be exposed
            type_: Some("ClusterIP".to_string()),
            cluster_ip: Some("None".to_string()),
            ports: Some(service_ports()),
            selector: Some(role_group_selector_labels(
                hello,
                APP_NAME,
                &rolegroup.role,
                &rolegroup.role_group,
            )),
            publish_not_ready_addresses: Some(true),
            ..ServiceSpec::default()
        }),
        status: None,
    })
}

/// The rolegroup [`StatefulSet`] runs the rolegroup, as configured by the administrator.
///
/// The [`Pod`](`stackable_operator::k8s_openapi::api::core::v1::Pod`)s are accessible through the
/// corresponding [`Service`] (from [`build_rolegroup_service`]).
fn build_server_rolegroup_statefulset(
    hello: &HelloCluster,
    resolved_product_image: &ResolvedProductImage,
    rolegroup_ref: &RoleGroupRef<HelloCluster>,
    metastore_config: &HashMap<PropertyNameKind, BTreeMap<String, String>>,
    merged_config: &ServerConfig,
    sa_name: &str,
) -> Result<StatefulSet> {
    // TODO this function still needs to be checked
    let rolegroup = hello
        .spec
        .servers
        .as_ref()
        .context(NoServerRoleSnafu)?
        .role_groups
        .get(&rolegroup_ref.role_group);
    let mut container_builder =
        ContainerBuilder::new(APP_NAME).context(FailedToCreateHelloContainerSnafu {
            name: APP_NAME.to_string(),
        })?;

    for (property_name_kind, config) in metastore_config {
        match property_name_kind {
            PropertyNameKind::Env => {
                // overrides
                for (property_name, property_value) in config {
                    if property_name.is_empty() {
                        warn!("Received empty property_name for ENV... skipping");
                        continue;
                    }
                    container_builder.add_env_var(property_name, property_value);
                }
            }
            _ => {}
        }
    }

    let mut pod_builder = PodBuilder::new();

    let container_hello = container_builder
        .command(vec![
            "nginx".to_string(),
            "-c".to_string(),
            format!("{}/{}", STACKABLE_CONFIG_MOUNT_DIR, NGINX_CONF),
        ])
        .image_from_product_image(resolved_product_image)
        .add_volume_mount(STACKABLE_CONFIG_DIR_NAME, STACKABLE_CONFIG_DIR)
        .add_volume_mount(STACKABLE_CONFIG_MOUNT_DIR_NAME, STACKABLE_CONFIG_MOUNT_DIR)
        .add_volume_mount(STACKABLE_LOG_DIR_NAME, STACKABLE_LOG_DIR)
        .add_volume_mount(
            STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME,
            STACKABLE_LOG_CONFIG_MOUNT_DIR,
        )
        .add_container_port(HTTP_PORT_NAME, HTTP_PORT.into())
        .resources(merged_config.resources.clone().into())
        .readiness_probe(Probe {
            initial_delay_seconds: Some(10),
            period_seconds: Some(10),
            failure_threshold: Some(5),
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::String(HTTP_PORT_NAME.to_string()),
                ..TCPSocketAction::default()
            }),
            ..Probe::default()
        })
        .liveness_probe(Probe {
            initial_delay_seconds: Some(30),
            period_seconds: Some(10),
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::String(HTTP_PORT_NAME.to_string()),
                ..TCPSocketAction::default()
            }),
            ..Probe::default()
        })
        .build();

    pod_builder
        .metadata_builder(|m| {
            m.with_recommended_labels(build_recommended_labels(
                hello,
                &resolved_product_image.app_version_label,
                &rolegroup_ref.role,
                &rolegroup_ref.role_group,
            ))
        })
        .image_pull_secrets_from_product_image(resolved_product_image)
        .add_container(container_hello)
        .add_volume(Volume {
            name: STACKABLE_CONFIG_DIR_NAME.to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                medium: None,
                size_limit: Some(Quantity("10Mi".to_string())),
            }),
            ..Volume::default()
        })
        .add_volume(stackable_operator::k8s_openapi::api::core::v1::Volume {
            name: STACKABLE_CONFIG_MOUNT_DIR_NAME.to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: Some(rolegroup_ref.object_name()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .add_volume(Volume {
            name: STACKABLE_LOG_DIR_NAME.to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                medium: None,
                size_limit: Some(Quantity(format!("{LOG_VOLUME_SIZE_IN_MIB}Mi"))),
            }),
            ..Volume::default()
        })
        .affinity(&merged_config.affinity)
        .service_account_name(sa_name);

    // .security_context(
    //     PodSecurityContextBuilder::new()
    //         .run_as_user(HELLO_UID)
    //         .run_as_group(0)
    //         .fs_group(1000)
    //         .build(),
    // )

    if let Some(ContainerLogConfig {
        choice:
            Some(ContainerLogConfigChoice::Custom(CustomContainerLogConfig {
                custom: ConfigMapLogConfig { config_map },
            })),
    }) = merged_config.logging.containers.get(&Container::Hello)
    {
        pod_builder.add_volume(Volume {
            name: STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME.to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: Some(config_map.into()),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        });
    } else {
        pod_builder.add_volume(Volume {
            name: STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME.to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: Some(rolegroup_ref.object_name()),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        });
    }

    if merged_config.logging.enable_vector_agent {
        pod_builder.add_container(product_logging::framework::vector_container(
            resolved_product_image,
            STACKABLE_CONFIG_DIR_NAME,
            STACKABLE_LOG_DIR_NAME,
            merged_config.logging.containers.get(&Container::Vector),
        ));
    }

    Ok(StatefulSet {
        metadata: ObjectMetaBuilder::new()
            .name_and_namespace(hello)
            .name(&rolegroup_ref.object_name())
            .ownerreference_from_resource(hello, None, Some(true))
            .context(ObjectMissingMetadataForOwnerRefSnafu)?
            .with_recommended_labels(build_recommended_labels(
                hello,
                &resolved_product_image.app_version_label,
                &rolegroup_ref.role,
                &rolegroup_ref.role_group,
            ))
            .build(),
        spec: Some(StatefulSetSpec {
            pod_management_policy: Some("Parallel".to_string()),
            replicas: rolegroup.and_then(|rg| rg.replicas).map(i32::from),
            selector: LabelSelector {
                match_labels: Some(role_group_selector_labels(
                    hello,
                    APP_NAME,
                    &rolegroup_ref.role,
                    &rolegroup_ref.role_group,
                )),
                ..LabelSelector::default()
            },
            service_name: rolegroup_ref.object_name(),
            template: pod_builder.build_template(),
            volume_claim_templates: Some(vec![merged_config
                .resources
                .storage
                .data
                .build_pvc("data", Some(vec!["ReadWriteOnce"]))]),
            ..StatefulSetSpec::default()
        }),
        status: None,
    })
}

pub fn error_policy(_obj: Arc<HelloCluster>, _error: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(Duration::from_secs(5))
}

fn service_ports() -> Vec<ServicePort> {
    vec![ServicePort {
        name: Some(HTTP_PORT_NAME.to_string()),
        port: HTTP_PORT.into(),
        protocol: Some("TCP".to_string()),
        ..ServicePort::default()
    }]
}

/// Creates recommended `ObjectLabels` to be used in deployed resources
pub fn build_recommended_labels<'a, T>(
    owner: &'a T,
    app_version: &'a str,
    role: &'a str,
    role_group: &'a str,
) -> ObjectLabels<'a, T> {
    ObjectLabels {
        owner,
        app_name: APP_NAME,
        app_version,
        operator_name: OPERATOR_NAME,
        controller_name: HELLO_CONTROLLER_NAME,
        role,
        role_group,
    }
}
