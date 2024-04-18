use stackable_operator::{
    commons::affinity::{affinity_between_role_pods, StackableAffinityFragment},
    k8s_openapi::api::core::v1::PodAntiAffinity,
};

use crate::{crd::HelloRole, APP_NAME};

pub fn get_affinity(cluster_name: &str, role: &HelloRole) -> StackableAffinityFragment {
    StackableAffinityFragment {
        pod_affinity: None,
        pod_anti_affinity: Some(PodAntiAffinity {
            preferred_during_scheduling_ignored_during_execution: Some(vec![
                affinity_between_role_pods(APP_NAME, cluster_name, &role.to_string(), 70),
            ]),
            required_during_scheduling_ignored_during_execution: None,
        }),
        node_affinity: None,
        node_selector: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::crd::HelloCluster;
    use rstest::rstest;
    use stackable_operator::{
        commons::affinity::StackableAffinity,
        k8s_openapi::{
            api::core::v1::{PodAffinityTerm, PodAntiAffinity, WeightedPodAffinityTerm},
            apimachinery::pkg::apis::meta::v1::LabelSelector,
        },
    };

    #[rstest]
    #[case(HelloRole::Server)]
    fn test_affinity_defaults(#[case] role: HelloRole) {
        let input = r#"
        apiVersion: hello-world.stackable.tech/v1alpha1
        kind: HelloCluster
        metadata:
          name: hello-world
        spec:
          image:
            productVersion: 0.1.0
          recipient: "Stackable"
          color: "blue"
          clusterConfig:
              listenerClass: external-unstable
          servers:
            roleGroups:
              default:
                replicas: 1
        "#;
        let hello: HelloCluster = serde_yaml::from_str(input).expect("illegal test input");
        let merged_config = hello
            .merged_config(&role, &role.rolegroup_ref(&hello, "default"))
            .unwrap();

        assert_eq!(
            merged_config.affinity,
            StackableAffinity {
                pod_affinity: None,
                pod_anti_affinity: Some(PodAntiAffinity {
                    preferred_during_scheduling_ignored_during_execution: Some(vec![
                        WeightedPodAffinityTerm {
                            pod_affinity_term: PodAffinityTerm {
                                label_selector: Some(LabelSelector {
                                    match_expressions: None,
                                    match_labels: Some(BTreeMap::from([
                                        (
                                            "app.kubernetes.io/name".to_string(),
                                            "hello-world".to_string(),
                                        ),
                                        (
                                            "app.kubernetes.io/instance".to_string(),
                                            "hello-world".to_string(),
                                        ),
                                        (
                                            "app.kubernetes.io/component".to_string(),
                                            role.to_string(),
                                        )
                                    ]))
                                }),
                                namespace_selector: None,
                                namespaces: None,
                                topology_key: "kubernetes.io/hostname".to_string(),
                            },
                            weight: 70
                        }
                    ]),
                    required_during_scheduling_ignored_during_execution: None,
                }),
                node_affinity: None,
                node_selector: None,
            }
        );
    }
}
