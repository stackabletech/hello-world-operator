use crate::affinity::get_affinity;
use indoc::formatdoc;
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
pub const STACKABLE_CONFIG_MOUNT_DIR: &str = "/stackable/mount/config";
pub const STACKABLE_CONFIG_MOUNT_DIR_NAME: &str = "config-mount";
pub const STACKABLE_LOG_DIR: &str = "/stackable/log";
pub const STACKABLE_LOG_DIR_NAME: &str = "log";
pub const STACKABLE_LOG_CONFIG_MOUNT_DIR: &str = "/stackable/mount/log-config";
pub const STACKABLE_LOG_CONFIG_MOUNT_DIR_NAME: &str = "log-config-mount";
// config file names
pub const HIVE_SITE_XML: &str = "hive-site.xml";
pub const HIVE_ENV_SH: &str = "hive-env.sh";
pub const HIVE_LOG4J2_PROPERTIES: &str = "hive-log4j2.properties";
// default ports
pub const HIVE_PORT_NAME: &str = "hive";
pub const HIVE_PORT: u16 = 9083;
pub const METRICS_PORT_NAME: &str = "metrics";
pub const METRICS_PORT: u16 = 9084;
// certificates and trust stores
pub const STACKABLE_TRUST_STORE: &str = "/stackable/truststore.p12";
pub const STACKABLE_TRUST_STORE_PASSWORD: &str = "changeit";
// metastore opts
pub const HIVE_METASTORE_HADOOP_OPTS: &str = "HIVE_METASTORE_HADOOP_OPTS";

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
    /// The Hive metastore image to use
    pub image: ProductImage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Role<MetaStoreConfigFragment>>,
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
    Hive,
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
pub struct MetastoreStorageConfig {
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
pub struct MetaStoreConfig {
    /// The location of default database for the Hive warehouse.
    /// Maps to the `hive.metastore.warehouse.dir` setting.
    pub warehouse_dir: Option<String>,
    #[fragment_attrs(serde(default))]
    pub resources: Resources<MetastoreStorageConfig, NoRuntimeLimits>,
    #[fragment_attrs(serde(default))]
    pub logging: Logging<Container>,
    #[fragment_attrs(serde(default))]
    pub affinity: StackableAffinity,
}

impl MetaStoreConfig {
    // metastore
    pub const CONNECTION_URL: &'static str = "javax.jdo.option.ConnectionURL";
    pub const CONNECTION_DRIVER_NAME: &'static str = "javax.jdo.option.ConnectionDriverName";
    pub const CONNECTION_USER_NAME: &'static str = "javax.jdo.option.ConnectionUserName";
    pub const CONNECTION_PASSWORD: &'static str = "javax.jdo.option.ConnectionPassword";
    pub const METASTORE_METRICS_ENABLED: &'static str = "hive.metastore.metrics.enabled";
    pub const METASTORE_WAREHOUSE_DIR: &'static str = "hive.metastore.warehouse.dir";
    pub const DB_TYPE_CLI: &'static str = "dbType";
    // S3
    pub const S3_ENDPOINT: &'static str = "fs.s3a.endpoint";
    pub const S3_ACCESS_KEY: &'static str = "fs.s3a.access.key";
    pub const S3_SECRET_KEY: &'static str = "fs.s3a.secret.key";
    pub const S3_SSL_ENABLED: &'static str = "fs.s3a.connection.ssl.enabled";
    pub const S3_PATH_STYLE_ACCESS: &'static str = "fs.s3a.path.style.access";

    fn default_config(cluster_name: &str, role: &HelloRole) -> MetaStoreConfigFragment {
        MetaStoreConfigFragment {
            warehouse_dir: None,
            resources: ResourcesFragment {
                cpu: CpuLimitsFragment {
                    min: Some(Quantity("200m".to_owned())),
                    max: Some(Quantity("4".to_owned())),
                },
                memory: MemoryLimitsFragment {
                    limit: Some(Quantity("2Gi".to_owned())),
                    runtime_limits: NoRuntimeLimitsFragment {},
                },
                storage: MetastoreStorageConfigFragment {
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

impl Configuration for MetaStoreConfigFragment {
    type Configurable = HelloCluster;

    fn compute_env(
        &self,
        _hive: &Self::Configurable,
        _role_name: &str,
    ) -> Result<BTreeMap<String, Option<String>>, ConfigError> {
        let mut result = BTreeMap::new();

        result.insert(
            HIVE_METASTORE_HADOOP_OPTS.to_string(),
            Some(formatdoc! {"
                    -javaagent:/stackable/jmx/jmx_prometheus_javaagent-0.16.1.jar={METRICS_PORT}:/stackable/jmx/jmx_hive_config.yaml
                    -Djavax.net.ssl.trustStore={STACKABLE_TRUST_STORE}
                    -Djavax.net.ssl.trustStorePassword={STACKABLE_TRUST_STORE_PASSWORD}
                    -Djavax.net.ssl.trustStoreType=pkcs12"}
                )
            );

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
        _hello: &Self::Configurable,
        _role_name: &str,
        file: &str,
    ) -> Result<BTreeMap<String, Option<String>>, ConfigError> {
        let mut result = BTreeMap::new();

        match file {
            HIVE_SITE_XML => {
                if let Some(warehouse_dir) = &self.warehouse_dir {
                    result.insert(
                        MetaStoreConfig::METASTORE_WAREHOUSE_DIR.to_string(),
                        Some(warehouse_dir.to_string()),
                    );
                };

                result.insert(
                    MetaStoreConfig::METASTORE_METRICS_ENABLED.to_string(),
                    Some("true".to_string()),
                );
            }
            HIVE_ENV_SH => {}
            _ => {}
        }

        Ok(result)
    }
}

#[derive(Clone, Default, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloClusterStatus {
    /// An opaque value that changes every time a discovery detail does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_hash: Option<String>,
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
    pub fn metastore_role_service_name(&self) -> Option<&str> {
        self.metadata.name.as_deref()
    }

    /// Metadata about a metastore rolegroup
    pub fn metastore_rolegroup_ref(
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
            .server
            .iter()
            .flat_map(|role| &role.role_groups)
            // Order rolegroups consistently, to avoid spurious downstream rewrites
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .flat_map(move |(rolegroup_name, rolegroup)| {
                let rolegroup_ref = self.metastore_rolegroup_ref(rolegroup_name);
                let ns = ns.clone();
                (0..rolegroup.replicas.unwrap_or(0)).map(move |i| PodRef {
                    namespace: ns.clone(),
                    role_group_service_name: rolegroup_ref.object_name(),
                    pod_name: format!("{}-{}", rolegroup_ref.object_name(), i),
                })
            }))
    }

    pub fn get_role(&self, role: &HelloRole) -> Option<&Role<MetaStoreConfigFragment>> {
        match role {
            HelloRole::Server => self.spec.server.as_ref(),
        }
    }

    /// Retrieve and merge resource configs for role and role groups
    pub fn merged_config(
        &self,
        role: &HelloRole,
        role_group: &str,
    ) -> Result<MetaStoreConfig, Error> {
        // Initialize the result with all default values as baseline
        let conf_defaults = MetaStoreConfig::default_config(&self.name_any(), role);

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

/// Reference to a single `Pod` that is a component of a [`HiveCluster`]
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
