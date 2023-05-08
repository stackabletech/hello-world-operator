# Helm Chart for Stackable Operator for Hello World

This Helm Chart can be used to install Custom Resource Definitions and the Operator for Hello World provided by Stackable.

## Requirements

- Create a [Kubernetes Cluster](../Readme.md)
- Install [Helm](https://helm.sh/docs/intro/install/)

## Install the Stackable Operator for Hello World

```bash
# From the root of the operator repository
make compile-chart

helm install hello-world-operator deploy/helm/hello-world-operator
```

## Usage of the CRDs

The usage of this operator and its CRDs is described in the [documentation](https://docs.stackable.tech/hello/index.html)

The operator has example requests included in the [`/examples`](https://github.com/stackabletech/hello-world-operator/tree/main/examples) directory.

## Links

https://github.com/stackabletech/hello-world-operator
