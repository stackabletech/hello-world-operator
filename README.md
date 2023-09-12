<p align="center">
  <img width="150" src="./.readme/static/borrowed/Icon_Stackable.svg" alt="Stackable Logo"/>
</p>

<h1 align="center">Stackable Demo Operator - Hello World!</h1>

![Build Actions Status](https://ci.stackable.tech/buildStatus/icon?job=hello-world%2doperator%2dit%2dnightly&subject=Integration%20Tests)
[![Maintenance](https://img.shields.io/badge/Maintained%3F-yes-green.svg)](https://GitHub.com/stackabletech/hello-world-operator/graphs/commit-activity)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-green.svg)](https://docs.stackable.tech/home/stable/contributor/index.html)
[![License OSL3.0](https://img.shields.io/badge/license-OSL3.0-green)](./LICENSE)

[Documentation](https://docs.stackable.tech/hello-world/stable/index.html) | [Stackable Data Platform](https://stackable.tech/) | [Platform Docs](https://docs.stackable.tech/) | [Discussions](https://github.com/orgs/stackabletech/discussions) | [Discord](https://discord.gg/7kZ3BNnCAF)

This is a an example Kubernetes operator that runs a simple nginx instance. It serves as documentation of how a Stackable Operator works and it can also be a good starting point for building a new Operator.

Unlike the other stackable Operators, this one is not installable with Helm or stackablectl, as it is only for educational purposes.

... TODO Steps to get it to run etc.


Apply the CRD

    cargo run -- crd | kubectl apply -f -

Deploy the HelloCluster:

    kubectl apply -f hello.yaml && cargo run -- run

Connect:

    kubectl port-forward svc/hello-world 8080

reachable at localhost:8080


## About The Stackable Data Platform

This operator is written and maintained by [Stackable](https://www.stackable.tech) and it is part of a larger data platform.

![Stackable Data Platform Overview](./.readme/static/borrowed/sdp_overview.png)

Stackable makes it easy to operate data applications in any Kubernetes cluster.

The data platform offers many operators, new ones being added continuously. All our operators are designed and built to be easily interconnected and to be consistent to work with.

The [Stackable GmbH](https://stackable.tech/) is the company behind the Stackable Data Platform. Offering professional services, paid support plans and custom development.

We love open-source!

## Supported Platforms

We develop and test our operators on the following cloud platforms:

* AKS on Microsoft Azure
* EKS on Amazon Web Services (AWS)
* GKE on Google Cloud Platform (GCP)
* [IONOS Cloud Managed Kubernetes](https://cloud.ionos.com/managed/kubernetes)
* K3s
* Kubernetes (for an up to date list of supported versions please check the release notes in our [docs](https://docs.stackable.tech))
* Red Hat OpenShift


## Other Operators

These are the operators that are currently part of the Stackable Data Platform:

* [Stackable Operator for Apache Airflow](https://github.com/stackabletech/airflow-operator)
* [Stackable Operator for Apache Druid](https://github.com/stackabletech/druid-operator)
* [Stackable Operator for Apache HBase](https://github.com/stackabletech/hbase-operator)
* [Stackable Operator for Apache Hadoop HDFS](https://github.com/stackabletech/hdfs-operator)
* [Stackable Operator for Apache Hive](https://github.com/stackabletech/hive-operator)
* [Stackable Operator for Apache Kafka](https://github.com/stackabletech/kafka-operator)
* [Stackable Operator for Apache NiFi](https://github.com/stackabletech/nifi-operator)
* [Stackable Operator for Apache Spark](https://github.com/stackabletech/spark-k8s-operator)
* [Stackable Operator for Apache Superset](https://github.com/stackabletech/superset-operator)
* [Stackable Operator for Trino](https://github.com/stackabletech/trino-operator)
* [Stackable Operator for Apache ZooKeeper](https://github.com/stackabletech/zookeeper-operator)

And our internal operators:

* [Commons Operator](https://github.com/stackabletech/commons-operator)
* [Listener Operator](https://github.com/stackabletech/listener-operator)
* [OpenPolicyAgent Operator](https://github.com/stackabletech/opa-operator)
* [Secret Operator](https://github.com/stackabletech/secret-operator)

## Contributing

Contributions are welcome. Follow our [Contributors Guide](https://docs.stackable.tech/home/stable/contributor/index.html) to learn how you can contribute.

## License

[Open Software License version 3.0](./LICENSE).

## Support

Get started with the community edition! If you want professional support, [we offer subscription plans and custom licensing](https://stackable.tech/en/plans/).
