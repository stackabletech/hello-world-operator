# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [24.7.0] - 2024-07-24

### Changed

- Remove the "nameOverride" chart property and make naming of k8s objects
  consistent with other operators ([#78]).
- Bump `stackable-operator` to `0.70.0`, `product-config` to `0.7.0`, and other dependencies ([#104]).

[#78]: https://github.com/stackabletech/hello-world-operator/pull/78
[#104]: https://github.com/stackabletech/hello-world-operator/pull/104

## [24.3.0] - 2024-03-20

### Added

- Helm: support labels in values.yaml ([#48]).

[#48]: https://github.com/stackabletech/hello-world-operator/pull/48

## [23.11.0] - 2023-11-24

### Added

- Set explicit resources on all containers ([#14]).
- Support `podOverrides` ([#15]).
- Increase the size limit of the log volumes ([#18])
- Configuration overrides for the JVM security properties, such as DNS caching ([#23]).
- Support PodDisruptionBudgets ([#27]).
- Support graceful shutdown ([#33]).

### Changed

- Default stackableVersion to operator version ([#21]).

[#14]: https://github.com/stackabletech/hello-world-operator/pull/14
[#15]: https://github.com/stackabletech/hello-world-operator/pull/15
[#18]: https://github.com/stackabletech/hello-world-operator/pull/18
[#21]: https://github.com/stackabletech/hello-world-operator/pull/21
[#23]: https://github.com/stackabletech/hello-world-operator/pull/23
[#27]: https://github.com/stackabletech/hello-world-operator/pull/27
[#33]: https://github.com/stackabletech/hello-world-operator/pull/33
