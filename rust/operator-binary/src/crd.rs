//! This file contains the definition of all the custom resources that this Operator manages.
//! In this case, it is only the `HelloCluster`.
//!
//! When writing a new Operator, this is often a good starting point. Edits made here will ripple
//! through the codebase, so it's easy to follow up from here.
use crate::affinity::get_affinity;
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    commons::{
        affinity::StackableAffinity,
        cluster_operation::ClusterOperation,
        product_image_selection::ProductImage,
        resources::{
            CpuLimitsFragment, MemoryLimitsFragment, NoRuntimeLimits, NoRuntimeLimitsFragment,
            PvcConfig, PvcConfigFragment, Resources, ResourcesFragment,
        },
    },
    config::{fragment, fragment::Fragment, fragment::ValidationError, merge::Merge},
    k8s_openapi::apimachinery::pkg::api::resource::Quantity,
    kube::{runtime::reflector::ObjectRef, CustomResource, ResourceExt},
    product_config_utils::{ConfigError, Configuration},
    product_logging::{self, spec::Logging},
    role_utils::{Role, RoleGroup, RoleGroupRef},
    schemars::{self, JsonSchema},
    status::condition::{ClusterCondition, HasStatusCondition},
};
use std::collections::BTreeMap;
use strum::{Display, EnumIter};

pub const APP_NAME: &str = "hello";
// directories
pub const STACKABLE_CONFIG_DIR: &str = "/stackable/config";
pub const STACKABLE_CONFIG_DIR_NAME: &str = "config";
pub const STACKABLE_LOG_DIR: &str = "/stackable/log";
pub const STACKABLE_LOG_DIR_NAME: &str = "log";
pub const STACKABLE_LOG_CONFIG_MOUNT_DIR: &str = "/stackable/mount/log-config";
pub const STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME: &str = "log-config-mount";
// config file names
pub const APPLICATION_PROPERTIES: &str = "application.properties";
pub const LOGBACK_XML: &str = "logback.xml";
pub const HELLO_WORLD_LOG_FILE: &str = "hello-world.log4j.xml"; // the extension .log4j.xml is important!
                                                                // config properties
pub const SERVER_PORT: &str = "server.port";
pub const LOGGING_CONFIG: &str = "logging.config";
pub const GREETING_RECIPIENT: &str = "greeting.recipient";
pub const GREETING_COLOR: &str = "greeting.color";
// default ports
pub const HTTP_PORT_NAME: &str = "http";
pub const HTTP_PORT: u16 = 8080;

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("no metastore role configuration provided"))]
    MissingMetaStoreRole,
    #[snafu(display("fragment validation failure"))]
    FragmentValidationFailure { source: ValidationError },
}

#[derive(Clone, CustomResource, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "hello.stackable.tech",
    version = "v1alpha1",
    kind = "HelloCluster",
    plural = "helloclusters",
    shortname = "hello",
    status = "HelloClusterStatus",
    namespaced,
    crates(
        kube_core = "stackable_operator::kube::core",
        k8s_openapi = "stackable_operator::k8s_openapi",
        schemars = "stackable_operator::schemars"
    )
)]
pub struct HelloClusterSpec {
    /// General Hive metastore cluster settings
    pub cluster_config: HelloClusterConfig,
    /// Cluster operations like pause reconciliation or cluster stop.
    #[serde(default)]
    pub cluster_operation: ClusterOperation,
    /// The image to use. In this example this will be an nginx image
    pub image: ProductImage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<Role<ServerConfigFragment>>,
    pub recipient: String,
    pub color: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloClusterConfig {
    /// Name of the Vector aggregator discovery ConfigMap.
    /// It must contain the key `ADDRESS` with the address of the Vector aggregator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_aggregator_config_map_name: Option<String>,
    /// In the future this setting will control, which ListenerClass <https://docs.stackable.tech/home/stable/listener-operator/listenerclass.html>
    /// will be used to expose the service.
    /// Currently only a subset of the ListenerClasses are supported by choosing the type of the created Services
    /// by looking at the ListenerClass name specified,
    /// In a future release support for custom ListenerClasses will be introduced without a breaking change:
    ///
    /// * cluster-internal: Use a ClusterIP service
    ///
    /// * external-unstable: Use a NodePort service
    ///
    /// * external-stable: Use a LoadBalancer service
    #[serde(default)]
    pub listener_class: CurrentlySupportedListenerClasses,
}

// TODO: Temporary solution until listener-operator is finished
#[derive(Clone, Debug, Default, Display, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum CurrentlySupportedListenerClasses {
    #[default]
    #[serde(rename = "cluster-internal")]
    ClusterInternal,
    #[serde(rename = "external-unstable")]
    ExternalUnstable,
    #[serde(rename = "external-stable")]
    ExternalStable,
}

impl CurrentlySupportedListenerClasses {
    pub fn k8s_service_type(&self) -> String {
        match self {
            CurrentlySupportedListenerClasses::ClusterInternal => "ClusterIP".to_string(),
            CurrentlySupportedListenerClasses::ExternalUnstable => "NodePort".to_string(),
            CurrentlySupportedListenerClasses::ExternalStable => "LoadBalancer".to_string(),
        }
    }
}

#[derive(Display)]
#[strum(serialize_all = "camelCase")]
pub enum HelloRole {
    #[strum(serialize = "server")]
    Server,
}

#[derive(
    Clone,
    Debug,
    Deserialize,
    Display,
    Eq,
    EnumIter,
    JsonSchema,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum Container {
    Hello,
    Vector,
}

#[derive(Clone, Debug, Default, JsonSchema, PartialEq, Fragment)]
#[fragment_attrs(
    derive(
        Clone,
        Debug,
        Default,
        Deserialize,
        Merge,
        JsonSchema,
        PartialEq,
        Serialize
    ),
    serde(rename_all = "camelCase")
)]
pub struct ServerStorageConfig {
    #[fragment_attrs(serde(default))]
    pub data: PvcConfig,
}

#[derive(Clone, Debug, Default, Fragment, JsonSchema, PartialEq)]
#[fragment_attrs(
    derive(
        Clone,
        Debug,
        Default,
        Deserialize,
        Merge,
        JsonSchema,
        PartialEq,
        Serialize
    ),
    serde(rename_all = "camelCase")
)]
pub struct ServerConfig {
    #[fragment_attrs(serde(default))]
    pub resources: Resources<ServerStorageConfig, NoRuntimeLimits>,
    #[fragment_attrs(serde(default))]
    pub logging: Logging<Container>,
    #[fragment_attrs(serde(default))]
    pub affinity: StackableAffinity,
}

impl ServerConfig {
    fn default_config(cluster_name: &str, role: &HelloRole) -> ServerConfigFragment {
        ServerConfigFragment {
            resources: ResourcesFragment {
                cpu: CpuLimitsFragment {
                    min: Some(Quantity("200m".to_owned())),
                    max: Some(Quantity("4".to_owned())),
                },
                memory: MemoryLimitsFragment {
                    limit: Some(Quantity("2Gi".to_owned())),
                    runtime_limits: NoRuntimeLimitsFragment {},
                },
                storage: ServerStorageConfigFragment {
                    data: PvcConfigFragment {
                        capacity: Some(Quantity("2Gi".to_owned())),
                        storage_class: None,
                        selectors: None,
                    },
                },
            },
            logging: product_logging::spec::default_logging(),
            affinity: get_affinity(cluster_name, role),
        }
    }
}

// TODO: Temporary solution until listener-operator is finished
#[derive(Clone, Debug, Display, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum ServiceType {
    NodePort,
    ClusterIP,
}

impl Default for ServiceType {
    fn default() -> Self {
        Self::NodePort
    }
}

impl Configuration for ServerConfigFragment {
    type Configurable = HelloCluster;

    fn compute_env(
        &self,
        _hello: &Self::Configurable,
        _role_name: &str,
    ) -> Result<BTreeMap<String, Option<String>>, ConfigError> {
        let result = BTreeMap::new();
        // no ENV args necessary
        Ok(result)
    }

    fn compute_cli(
        &self,
        _hello: &Self::Configurable,
        _role_name: &str,
    ) -> Result<BTreeMap<String, Option<String>>, ConfigError> {
        let result = BTreeMap::new();
        // No CLI args necessary
        Ok(result)
    }

    fn compute_files(
        &self,
        hello: &Self::Configurable,
        _role_name: &str,
        file: &str,
    ) -> Result<BTreeMap<String, Option<String>>, ConfigError> {
        let mut result = BTreeMap::new();

        match file {
            APPLICATION_PROPERTIES => {
                result.insert(
                    GREETING_RECIPIENT.to_owned(),
                    Some(hello.spec.recipient.to_owned()),
                );
                result.insert(GREETING_COLOR.to_owned(), Some(hello.spec.color.to_owned()));
                result.insert(SERVER_PORT.to_owned(), Some(HTTP_PORT.to_string()));
                result.insert(
                    LOGGING_CONFIG.to_owned(),
                    Some(format!("{}/{}", STACKABLE_CONFIG_DIR, LOGBACK_XML)),
                );
            }
            _ => {}
        }

        Ok(result)
    }
}

#[derive(Clone, Default, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloClusterStatus {
    pub conditions: Vec<ClusterCondition>,
}

impl HasStatusCondition for HelloCluster {
    fn conditions(&self) -> Vec<ClusterCondition> {
        match &self.status {
            Some(status) => status.conditions.clone(),
            None => vec![],
        }
    }
}

#[derive(Debug, Snafu)]
#[snafu(display("object has no namespace associated"))]
pub struct NoNamespaceError;

impl HelloCluster {
    /// The name of the role-level load-balanced Kubernetes `Service`
    pub fn server_role_service_name(&self) -> Option<&str> {
        self.metadata.name.as_deref()
    }

    /// Metadata about a server rolegroup
    pub fn server_rolegroup_ref(
        &self,
        group_name: impl Into<String>,
    ) -> RoleGroupRef<HelloCluster> {
        RoleGroupRef {
            cluster: ObjectRef::from_obj(self),
            role: HelloRole::Server.to_string(),
            role_group: group_name.into(),
        }
    }

    /// List all pods expected to form the cluster
    ///
    /// We try to predict the pods here rather than looking at the current cluster state in order to
    /// avoid instance churn.
    pub fn pods(&self) -> Result<impl Iterator<Item = PodRef> + '_, NoNamespaceError> {
        let ns = self.metadata.namespace.clone().context(NoNamespaceSnafu)?;
        Ok(self
            .spec
            .servers
            .iter()
            .flat_map(|role| &role.role_groups)
            // Order rolegroups consistently, to avoid spurious downstream rewrites
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .flat_map(move |(rolegroup_name, rolegroup)| {
                let rolegroup_ref = self.server_rolegroup_ref(rolegroup_name);
                let ns = ns.clone();
                (0..rolegroup.replicas.unwrap_or(0)).map(move |i| PodRef {
                    namespace: ns.clone(),
                    role_group_service_name: rolegroup_ref.object_name(),
                    pod_name: format!("{}-{}", rolegroup_ref.object_name(), i),
                })
            }))
    }

    pub fn get_role(&self, role: &HelloRole) -> Option<&Role<ServerConfigFragment>> {
        match role {
            HelloRole::Server => self.spec.servers.as_ref(),
        }
    }

    /// Retrieve and merge resource configs for role and role groups
    pub fn merged_config(&self, role: &HelloRole, role_group: &str) -> Result<ServerConfig, Error> {
        // Initialize the result with all default values as baseline
        let conf_defaults = ServerConfig::default_config(&self.name_any(), role);

        let role = self.get_role(role).context(MissingMetaStoreRoleSnafu)?;

        // Retrieve role resource config
        let mut conf_role = role.config.config.to_owned();

        // Retrieve rolegroup specific resource config
        let mut conf_rolegroup = role
            .role_groups
            .get(role_group)
            .map(|rg| rg.config.config.clone())
            .unwrap_or_default();

        if let Some(RoleGroup {
            selector: Some(selector),
            ..
        }) = role.role_groups.get(role_group)
        {
            // Migrate old `selector` attribute, see ADR 26 affinities.
            // TODO Can be removed after support for the old `selector` field is dropped.
            #[allow(deprecated)]
            conf_rolegroup.affinity.add_legacy_selector(selector);
        }

        // Merge more specific configs into default config
        // Hierarchy is:
        // 1. RoleGroup
        // 2. Role
        // 3. Default
        conf_role.merge(&conf_defaults);
        conf_rolegroup.merge(&conf_role);

        tracing::debug!("Merged config: {:?}", conf_rolegroup);
        fragment::validate(conf_rolegroup).context(FragmentValidationFailureSnafu)
    }
}

/// Reference to a single `Pod` that is a component of a [`HelloCluster`]
/// Used for service discovery.
pub struct PodRef {
    pub namespace: String,
    pub role_group_service_name: String,
    pub pod_name: String,
}

impl PodRef {
    pub fn fqdn(&self) -> String {
        format!(
            "{}.{}.{}.svc.cluster.local",
            self.pod_name, self.role_group_service_name, self.namespace
        )
    }
}
