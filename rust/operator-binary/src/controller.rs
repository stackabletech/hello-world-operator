//! Ensures that `Pod`s are configured and running for each [`HelloCluster`]
use product_config::{
    self, types::PropertyNameKind, writer::to_java_properties_string, ProductConfigManager,
};
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    builder::{
        resources::ResourceRequirementsBuilder, ConfigMapBuilder, ContainerBuilder,
        ObjectMetaBuilder, PodBuilder,
    },
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
        apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString},
        DeepMerge,
    },
    kube::{runtime::controller::Action, Resource, ResourceExt},
    kvp::{Labels, ObjectLabels},
    logging::controller::ReconcilerError,
    memory::{BinaryMultiple, MemoryQuantity},
    product_config_utils::{transform_all_roles_to_config, validate_all_roles_and_groups_config},
    product_logging::{
        self,
        framework::{create_vector_shutdown_file_command, remove_vector_shutdown_file_command},
        spec::{
            ConfigMapLogConfig, ContainerLogConfig, ContainerLogConfigChoice,
            CustomContainerLogConfig,
        },
    },
    role_utils::{GenericRoleConfig, RoleGroupRef},
    status::condition::{
        compute_conditions, operations::ClusterOperationsConditionBuilder,
        statefulset::StatefulSetConditionBuilder,
    },
    utils::COMMON_BASH_TRAP_FUNCTIONS,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use strum::EnumDiscriminants;
use tracing::warn;

use crate::crd::{
    Container, HelloCluster, HelloClusterStatus, HelloConfig, HelloRole, APPLICATION_PROPERTIES,
    APP_NAME, HTTP_PORT, HTTP_PORT_NAME, JVM_SECURITY_PROPERTIES, STACKABLE_CONFIG_DIR,
    STACKABLE_CONFIG_DIR_NAME, STACKABLE_LOG_CONFIG_MOUNT_DIR, STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME,
    STACKABLE_LOG_DIR, STACKABLE_LOG_DIR_NAME,
};
use crate::operations::{graceful_shutdown::add_graceful_shutdown_config, pdb::add_pdbs};
use crate::product_logging::{extend_role_group_config_map, resolve_vector_aggregator_address};
use crate::OPERATOR_NAME;

pub const HELLO_CONTROLLER_NAME: &str = "hellocluster";
const DOCKER_IMAGE_BASE_NAME: &str = "hello";

pub const MAX_LOG_FILES_SIZE: MemoryQuantity = MemoryQuantity {
    value: 10.0,
    unit: BinaryMultiple::Mebi,
};

pub struct Ctx {
    pub client: stackable_operator::client::Client,
    pub product_config: ProductConfigManager,
}

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(strum::IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("internal operator failure"))]
    InternalOperatorFailure { source: crate::crd::Error },
    #[snafu(display("object defines no namespace"))]
    ObjectHasNoNamespace,
    #[snafu(display("object defines no hello role"))]
    NoServerRole,
    #[snafu(display("failed to calculate global service name"))]
    GlobalServiceNameNotFound,
    #[snafu(display("failed to apply global Service"))]
    ApplyRoleService {
        source: stackable_operator::error::Error,
    },
    #[snafu(display("failed to apply Service for {rolegroup}"))]
    ApplyRoleGroupService {
        source: stackable_operator::error::Error,
        rolegroup: RoleGroupRef<HelloCluster>,
    },
    #[snafu(display("failed to format runtime properties"))]
    PropertiesWriteError {
        source: product_config::writer::PropertiesWriterError,
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
    #[snafu(display("failed to update status"))]
    ApplyStatus {
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
    #[snafu(display(
        "failed to serialize [{JVM_SECURITY_PROPERTIES}] for group {}",
        rolegroup
    ))]
    JvmSecurityProperties {
        source: product_config::writer::PropertiesWriterError,
        rolegroup: String,
    },
    #[snafu(display("failed to create PodDisruptionBudget"))]
    FailedToCreatePdb {
        source: crate::operations::pdb::Error,
    },
    #[snafu(display("failed to configure graceful shutdown"))]
    GracefulShutdown {
        source: crate::operations::graceful_shutdown::Error,
    },

    #[snafu(display("failed to build Labels"))]
    LabelBuild {
        source: stackable_operator::kvp::LabelError,
    },

    #[snafu(display("failed to build Metadata"))]
    MetadataBuild {
        source: stackable_operator::builder::ObjectMetaBuilderError,
    },

    #[snafu(display("failed to get required Labels"))]
    GetRequiredLabels {
        source:
            stackable_operator::kvp::KeyValuePairError<stackable_operator::kvp::LabelValueError>,
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
    let resolved_product_image: ResolvedProductImage = hello
        .spec
        .image
        .resolve(DOCKER_IMAGE_BASE_NAME, crate::built_info::CARGO_PKG_VERSION);
    let hello_role = HelloRole::Server;

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
                        PropertyNameKind::File(APPLICATION_PROPERTIES.to_string()),
                        PropertyNameKind::File(JVM_SECURITY_PROPERTIES.to_string()),
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
        cluster_resources
            .get_required_labels()
            .context(GetRequiredLabelsSnafu)?,
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
        let role_group_ref = hello.server_rolegroup_ref(rolegroup_name);

        let config = hello
            .merged_config(&HelloRole::Server, &role_group_ref)
            .context(FailedToResolveResourceConfigSnafu)?;

        let rg_service = build_rolegroup_service(&hello, &resolved_product_image, &role_group_ref)?;
        let rg_configmap = build_server_rolegroup_config_map(
            &hello,
            &resolved_product_image,
            &role_group_ref,
            rolegroup_config,
            &config,
            vector_aggregator_address.as_deref(),
        )?;
        let rg_statefulset = build_server_rolegroup_statefulset(
            &hello,
            &resolved_product_image,
            &hello_role,
            &role_group_ref,
            rolegroup_config,
            &config,
            &rbac_sa.name_any(),
        )?;

        cluster_resources
            .add(client, rg_service)
            .await
            .context(ApplyRoleGroupServiceSnafu {
                rolegroup: role_group_ref.clone(),
            })?;

        cluster_resources
            .add(client, rg_configmap)
            .await
            .context(ApplyRoleGroupConfigSnafu {
                rolegroup: role_group_ref.clone(),
            })?;

        ss_cond_builder.add(
            cluster_resources
                .add(client, rg_statefulset)
                .await
                .context(ApplyRoleGroupStatefulSetSnafu {
                    rolegroup: role_group_ref.clone(),
                })?,
        );
    }

    let role_config = hello.role_config(&hello_role);
    if let Some(GenericRoleConfig {
        pod_disruption_budget: pdb,
    }) = role_config
    {
        add_pdbs(pdb, &hello, &hello_role, client, &mut cluster_resources)
            .await
            .context(FailedToCreatePdbSnafu)?;
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
            .context(MetadataBuildSnafu)?
            .build(),
        spec: Some(ServiceSpec {
            type_: Some(hello.spec.cluster_config.listener_class.k8s_service_type()),
            ports: Some(service_ports()),
            selector: Some(
                Labels::role_selector(hello, APP_NAME, &role_name)
                    .context(LabelBuildSnafu)?
                    .into(),
            ),
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
    merged_config: &HelloConfig,
    vector_aggregator_address: Option<&str>,
) -> Result<ConfigMap> {
    let mut application_properties = String::new();

    for (property_name_kind, config) in role_group_config {
        match property_name_kind {
            PropertyNameKind::File(file_name) if file_name == APPLICATION_PROPERTIES => {
                let transformed_config: BTreeMap<String, Option<String>> = config
                    .iter()
                    .map(|(k, v)| (k.clone(), Some(v.clone())))
                    .collect();

                application_properties = to_java_properties_string(transformed_config.iter())
                    .context(PropertiesWriteSnafu)?;
            }
            _ => {}
        }
    }

    // build JVM security properties from configOverrides.
    let jvm_sec_props: BTreeMap<String, Option<String>> = role_group_config
        .get(&PropertyNameKind::File(JVM_SECURITY_PROPERTIES.to_string()))
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, Some(v)))
        .collect();

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
                .context(MetadataBuildSnafu)?
                .build(),
        )
        .add_data(APPLICATION_PROPERTIES, application_properties)
        .add_data(
            JVM_SECURITY_PROPERTIES,
            to_java_properties_string(jvm_sec_props.iter()).with_context(|_| {
                JvmSecurityPropertiesSnafu {
                    rolegroup: rolegroup.role_group.clone(),
                }
            })?,
        );

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
            .context(MetadataBuildSnafu)?
            .build(),
        spec: Some(ServiceSpec {
            // Internal communication does not need to be exposed
            type_: Some("ClusterIP".to_string()),
            cluster_ip: Some("None".to_string()),
            ports: Some(service_ports()),
            selector: Some(
                Labels::role_group_selector(
                    hello,
                    APP_NAME,
                    &rolegroup.role,
                    &rolegroup.role_group,
                )
                .context(LabelBuildSnafu)?
                .into(),
            ),
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
    hello_role: &HelloRole,
    role_group_ref: &RoleGroupRef<HelloCluster>,
    rolegroup_config: &HashMap<PropertyNameKind, BTreeMap<String, String>>,
    merged_config: &HelloConfig,
    sa_name: &str,
) -> Result<StatefulSet> {
    // TODO this function still needs to be checked
    let role = hello
        .role(hello_role)
        .context(InternalOperatorFailureSnafu)?;
    let role_group = hello
        .role_group(role_group_ref)
        .context(InternalOperatorFailureSnafu)?;

    let mut container_builder =
        ContainerBuilder::new(APP_NAME).context(FailedToCreateHelloContainerSnafu {
            name: APP_NAME.to_string(),
        })?;

    for (property_name_kind, config) in rolegroup_config {
        if property_name_kind == &PropertyNameKind::Env {
            // overrides
            for (property_name, property_value) in config {
                if property_name.is_empty() {
                    warn!("Received empty property_name for ENV... skipping");
                    continue;
                }
                container_builder.add_env_var(property_name, property_value);
            }
        }
    }

    let command = [
        // graceful shutdown part
        COMMON_BASH_TRAP_FUNCTIONS.to_string(),
        remove_vector_shutdown_file_command(STACKABLE_LOG_DIR),
        "prepare_signal_handlers".to_string(),
        // run process
        format!("java -Djava.security.properties={STACKABLE_CONFIG_DIR}/{JVM_SECURITY_PROPERTIES} -jar hello-world.jar &"),
        // graceful shutdown part
        "wait_for_termination $!".to_string(),
        create_vector_shutdown_file_command(STACKABLE_LOG_DIR),
    ];

    let container_hello = container_builder
        .command(vec![
            "/bin/bash".to_string(),
            "-x".to_string(),
            "-euo".to_string(),
            "pipefail".to_string(),
            "-c".to_string(),
        ])
        .args(vec![command.join("\n")])
        .image_from_product_image(resolved_product_image)
        .add_volume_mount(STACKABLE_CONFIG_DIR_NAME, STACKABLE_CONFIG_DIR)
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

    let mut pod_builder = PodBuilder::new();
    add_graceful_shutdown_config(merged_config, &mut pod_builder).context(GracefulShutdownSnafu)?;

    let metadata = ObjectMetaBuilder::new()
        .with_recommended_labels(build_recommended_labels(
            hello,
            &resolved_product_image.app_version_label,
            &role_group_ref.role,
            &role_group_ref.role_group,
        ))
        .context(MetadataBuildSnafu)?
        .build();

    pod_builder
        .metadata(metadata)
        .image_pull_secrets_from_product_image(resolved_product_image)
        .add_container(container_hello)
        .add_volume(stackable_operator::k8s_openapi::api::core::v1::Volume {
            name: STACKABLE_CONFIG_DIR_NAME.to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: Some(role_group_ref.object_name()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .add_volume(Volume {
            name: STACKABLE_LOG_DIR_NAME.to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                medium: None,
                size_limit: Some(product_logging::framework::calculate_log_volume_size_limit(
                    &[MAX_LOG_FILES_SIZE],
                )),
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
                name: Some(role_group_ref.object_name()),
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
            ResourceRequirementsBuilder::new()
                .with_cpu_request("250m")
                .with_cpu_limit("500m")
                .with_memory_request("128Mi")
                .with_memory_limit("128Mi")
                .build(),
        ));
    }

    let mut pod_template = pod_builder.build_template();
    pod_template.merge_from(role.config.pod_overrides.clone());
    pod_template.merge_from(role_group.config.pod_overrides.clone());

    Ok(StatefulSet {
        metadata: ObjectMetaBuilder::new()
            .name_and_namespace(hello)
            .name(&role_group_ref.object_name())
            .ownerreference_from_resource(hello, None, Some(true))
            .context(ObjectMissingMetadataForOwnerRefSnafu)?
            .with_recommended_labels(build_recommended_labels(
                hello,
                &resolved_product_image.app_version_label,
                &role_group_ref.role,
                &role_group_ref.role_group,
            ))
            .context(MetadataBuildSnafu)?
            .build(),
        spec: Some(StatefulSetSpec {
            pod_management_policy: Some("Parallel".to_string()),
            replicas: role_group.replicas.map(i32::from),
            selector: LabelSelector {
                match_labels: Some(
                    Labels::role_group_selector(
                        hello,
                        APP_NAME,
                        &role_group_ref.role,
                        &role_group_ref.role_group,
                    )
                    .context(LabelBuildSnafu)?
                    .into(),
                ),
                ..LabelSelector::default()
            },
            service_name: role_group_ref.object_name(),
            template: pod_template,
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
