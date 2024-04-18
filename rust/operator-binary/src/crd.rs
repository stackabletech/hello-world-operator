//! This file contains the definition of all the custom resources that this Operator manages.
//! In this case, it is only the `HelloCluster`.
//!
//! When writing a new Operator, this is often a good starting point. Edits made here will ripple
//! through the codebase, so it's easy to follow up from here.
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
    role_utils::{GenericRoleConfig, Role, RoleGroup, RoleGroupRef},
    schemars::{self, JsonSchema},
    status::condition::{ClusterCondition, HasStatusCondition},
    time::Duration,
};
use std::{collections::BTreeMap, str::FromStr};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

use crate::affinity::get_affinity;

pub const APP_NAME: &str = "hello-world";
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
pub const JVM_SECURITY_PROPERTIES: &str = "security.properties";
// config properties
pub const SERVER_PORT: &str = "server.port";
pub const LOGGING_CONFIG: &str = "logging.config";
pub const GREETING_RECIPIENT: &str = "greeting.recipient";
pub const GREETING_COLOR: &str = "greeting.color";
// default ports
pub const HTTP_PORT_NAME: &str = "http";
pub const HTTP_PORT: u16 = 8080;

const DEFAULT_HELLO_WORLD_GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_minutes_unchecked(2);

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("fragment validation failure"))]
    FragmentValidationFailure { source: ValidationError },
    #[snafu(display("unknown role {role}. Should be one of {roles:?}"))]
    UnknownHelloRole {
        source: strum::ParseError,
        role: String,
        roles: Vec<String>,
    },
    #[snafu(display("the role {role} is not defined"))]
    CannotRetrieveHelloRole { role: String },
    #[snafu(display("the role group {role_group} is not defined"))]
    CannotRetrieveHelloRoleGroup { role_group: String },
}

#[derive(Clone, CustomResource, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "hello-world.stackable.tech",
    version = "v1alpha1",
    kind = "HelloCluster",
    plural = "hello-world-clusters",
    shortname = "hello-world",
    status = "HelloClusterStatus",
    namespaced,
    crates(
        kube_core = "stackable_operator::kube::core",
        k8s_openapi = "stackable_operator::k8s_openapi",
        schemars = "stackable_operator::schemars"
    )
)]
pub struct HelloClusterSpec {
    /// General Hello World cluster settings
    pub cluster_config: HelloClusterConfig,
    /// Cluster operations like pause reconciliation or cluster stop.
    #[serde(default)]
    pub cluster_operation: ClusterOperation,
    /// The image to use. In this example this will be an nginx image
    pub image: ProductImage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<Role<HelloConfigFragment>>,
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

#[derive(
    Clone,
    Debug,
    Deserialize,
    Display,
    EnumIter,
    Eq,
    Hash,
    JsonSchema,
    PartialEq,
    Serialize,
    EnumString,
)]
#[strum(serialize_all = "camelCase")]
pub enum HelloRole {
    #[strum(serialize = "server")]
    Server,
}

impl HelloRole {
    pub fn roles() -> Vec<String> {
        let mut roles = vec![];
        for role in Self::iter() {
            roles.push(role.to_string())
        }
        roles
    }

    /// Metadata about a rolegroup
    pub fn rolegroup_ref(
        &self,
        hello: &HelloCluster,
        group_name: impl Into<String>,
    ) -> RoleGroupRef<HelloCluster> {
        RoleGroupRef {
            cluster: ObjectRef::from_obj(hello),
            role: self.to_string(),
            role_group: group_name.into(),
        }
    }
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
pub struct HelloConfig {
    #[fragment_attrs(serde(default))]
    pub resources: Resources<ServerStorageConfig, NoRuntimeLimits>,
    #[fragment_attrs(serde(default))]
    pub logging: Logging<Container>,
    #[fragment_attrs(serde(default))]
    pub affinity: StackableAffinity,
    /// Time period Pods have to gracefully shut down, e.g. `30m`, `1h` or `2d`. Consult the operator documentation for details.
    #[fragment_attrs(serde(default))]
    pub graceful_shutdown_timeout: Option<Duration>,
}

impl HelloConfig {
    fn default_config(cluster_name: &str, role: &HelloRole) -> HelloConfigFragment {
        HelloConfigFragment {
            resources: ResourcesFragment {
                cpu: CpuLimitsFragment {
                    min: Some(Quantity("100m".to_owned())),
                    max: Some(Quantity("400m".to_owned())),
                },
                memory: MemoryLimitsFragment {
                    limit: Some(Quantity("256Mi".to_owned())),
                    runtime_limits: NoRuntimeLimitsFragment {},
                },
                storage: ServerStorageConfigFragment {
                    data: PvcConfigFragment {
                        capacity: Some(Quantity("256Mi".to_owned())),
                        storage_class: None,
                        selectors: None,
                    },
                },
            },
            logging: product_logging::spec::default_logging(),
            affinity: get_affinity(cluster_name, role),
            graceful_shutdown_timeout: Some(DEFAULT_HELLO_WORLD_GRACEFUL_SHUTDOWN_TIMEOUT),
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

impl Configuration for HelloConfigFragment {
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

        if file == APPLICATION_PROPERTIES {
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
    /// Returns a reference to the role. Raises an error if the role is not defined.
    pub fn role(&self, role_variant: &HelloRole) -> Result<&Role<HelloConfigFragment>, Error> {
        match role_variant {
            HelloRole::Server => self.spec.servers.as_ref(),
        }
        .with_context(|| CannotRetrieveHelloRoleSnafu {
            role: role_variant.to_string(),
        })
    }

    /// Returns a reference to the role group. Raises an error if the role or role group are not defined.
    pub fn role_group(
        &self,
        rolegroup_ref: &RoleGroupRef<HelloCluster>,
    ) -> Result<RoleGroup<HelloConfigFragment>, Error> {
        let role_variant =
            HelloRole::from_str(&rolegroup_ref.role).with_context(|_| UnknownHelloRoleSnafu {
                role: rolegroup_ref.role.to_owned(),
                roles: HelloRole::roles(),
            })?;
        let role = self.role(&role_variant)?;
        role.role_groups
            .get(&rolegroup_ref.role_group)
            .with_context(|| CannotRetrieveHelloRoleGroupSnafu {
                role_group: rolegroup_ref.role_group.to_owned(),
            })
            .cloned()
    }

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

    pub fn role_config(&self, role: &HelloRole) -> Option<&GenericRoleConfig> {
        match role {
            HelloRole::Server => self.spec.servers.as_ref().map(|s| &s.role_config),
        }
    }

    /// Retrieve and merge resource configs for role and role groups
    pub fn merged_config(
        &self,
        role: &HelloRole,
        rolegroup_ref: &RoleGroupRef<HelloCluster>,
    ) -> Result<HelloConfig, Error> {
        // Initialize the result with all default values as baseline
        let conf_defaults = HelloConfig::default_config(&self.name_any(), role);

        let role = self.role(role)?;
        let mut conf_role = role.config.config.to_owned();

        let role_group = self.role_group(rolegroup_ref)?;
        let mut conf_role_group = role_group.config.config.to_owned();

        // Merge more specific configs into default config
        // Hierarchy is:
        // 1. RoleGroup
        // 2. Role
        // 3. Default
        conf_role.merge(&conf_defaults);
        conf_role_group.merge(&conf_role);

        tracing::debug!("Merged config: {:?}", conf_role_group);
        fragment::validate(conf_role_group).context(FragmentValidationFailureSnafu)
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
