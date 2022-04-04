# Target Specification

This document specifies requirements for deployment targets for the
integration of Vector.

The key words “MUST”, “MUST NOT”, “REQUIRED”, “SHALL”, “SHALL NOT”, “SHOULD”,
“SHOULD NOT”, “RECOMMENDED”, “MAY”, and “OPTIONAL” in this document are to be
interpreted as described in [RFC 2119].

Other words, such as "agent", "aggregator", "node", and "service" are to be
interpreted as described in the [terminology document][terminology_document].

- [1. Introduction](#1-introduction)
- [2. Deployment Architectures](#2-deployment-architectures)
  - [3. Agent Architecture](#3-agent-architecture)
  - [4. Aggregator Architecture](#4-aggregator-architecture)
  - [5. Unified Architecture](#5-unified-architecture)
- [6. Hardening](#6-hardening)
- [7. Sizing & Scaling](#7-sizing--scaling)

## 1. Introduction

In its simplest form, installing Vector consists of downloading the binary and
making it executable, but leaves much to be desired for users looking to
integrate Vector in real-world production environments. To adhere with Vector's
["reduce decisions" design principle], Vector must also be opinionated about how
it's deployed, providing easy facilities for adopting Vector's
[reference architectures][reference_architectures],
[achieving high availability][high_availability], and [hardening][hardening]
Vector.

## 2. Deployment Architectures

When supporting a target, Vector must support them through the the paradigm of
architectures:

* Targets MUST support the [agent architecture][agent_architecture] by
  providing a single command that deploys Vector and achieves the
  [agent architecture requirements](#agent-architecture).
* Targets MUST support the [aggregator architecture][aggregator_architecture] by
  providing a single command that deploys Vector and achieves the
  [aggregator architecture requirements](#aggregator-architecture).
* Targets MAY support the [unified architecture][unified_architecture] by
  providing a single command that deploys Vector and achieves the
  [unified architecture requirements](#unified-architecture).

### 3. Agent Architecture

The [agent architecture][agent_architecture] deploys Vector on each individual
node for local data collection. The following requirements define support for
this architecture:

* Architecture
  * MUST deploy as a daemon on existing nodes, one Vector process per node.
  * MUST NOT deploy Vector aggregator nodes, since the Vector aggregator can be
    configured to assume agent responsibilities.
  * MUST deploy with Vector's [default agent configuration][default_agent_configuration]
    which largely covers the agent architecture
    [design recommendations][agent_architecture].
* Sizing
  * MUST deploy as a good infrastructure citizen, giving resource priority to
    other services on the same node.
  * SHOULD be limited to 2 vCPUs, MUST be overridable by the user.
  * SHOULD be limited to 2 GiB of memory per vCPU (4 GiB in this case), MUST be
    overridable by the user.
  * SHOULD be limited to 1 GiB of disk space, MUST be overridable by the user.

### 4. Aggregator Architecture

The [aggregator architecture][aggregator_architecture] deploys Vector onto
dedicated nodes for data aggregation. The following requirements define support
for this architecture:

* Architecture
  * MUST deploy as a stateful service on dedicated nodes, Vector is the only
    service on the node.
  * MUST deploy with a persistent disk that is available between deployments.
  * MUST deploy with Vector's [default aggregator configuration][default_aggregator_configuration]
    which largely covers the aggregator architecture
    [design recommendations][aggregator_architecture].
    in order to achieve durability with disk buffers and source checkpoints.
  * SHOULD deploy within one Cluster or VPC at a time.
  * Configured Vector ports, including non-default user configured ports,
    SHOULD be automatically accessible within the Cluster or VPC.
  * Configured Vector sources, including non-default user configured sources,
    SHOULD be automatically discoverable via target service discovery
    mechanisms.
* High Availability
  * SHOULD deploy across 2 nodes, MUST be overridable by the user.
  * SHOULD deploy across 2 availability zones, MUST be overridable by the user.
* Sizing
  * MUST deploy in a way that takes full advantage of all system resources.
    The Vector service should not be artificially limited with resource
    limiters such as cgroups.
  * SHOULD request 8 vCPUs, MUST be overridable by the user.
  * SHOULD request 2 GiB of memory per vCPU (16 GiB in this case), MUST be
    overridable by the user.
  * SHOULD request 6 GiB of disk space per vCPU (48 GiB in this case), MUST be
    overridable by the user.
  * SHOULD use the following instances if deployed in a listed cloud.
    * AWS SHOULD default to `c6g.2xlarge` instances with 48 GiB of EBS `io2`
      disk space.
    * Azure SHOULD default to `f8` instances with 48 GiB of standard SSD disk
      space.
    * GCP SHOULD default to `c2` instances with 8 vCPUS, 16 GiB of memory, and
      48 GiB of SSD persisted disk space.
* Scaling
  * Load-balancing?...
  * Autoscaling SHOULD be enabled by default driven by an 85% CPU utilization
    target over a rolling 5 minute window.
  * Autoscaling SHOULD have a stabilization period of 5 minutes.

### 5. Unified Architecture

The [unified architecture][unified_architecture] deploys Vector on each
individual node for local data collection, as well as onto dedicated nodes for
data aggregation. The requirements for both the [agent](#3-agent-architecture)
and the [aggregator](#4-aggregator-architecture) apply to this architecture.

## 6. Hardening

* Setup
  * An unprivileged Vector service account SHOULD be created upon installation
    for running the Vector process.
* Data hardening
  * Swap SHOULD be disabled to prevent in-flight data from leaking to disk.
    Swap would also make Vector prohibitively slow.
  * Vector's data directory SHOULD be read and write restricted to Vector's
    dedicated service account.
  * Core dumps SHOULD be prevented for the Vector process to prevent in flight
    data from leaking to disk.
* Process hardening
  * Vector's artifacts
    * All communication during the setup process, such as downloading Vector
      artifacts, MUST use encrypted channels.
    * Downloaded Vector artifacts MUST be verified against the provided
      checksum.
    * The latest Vector version SHOULD be downloaded unless otherwise specified
      by the user.
  * Vector's configuration
    * Vector's configuration directory SHOULD be read restricted to Vector's
      service account.
  * Vector's runtime
    * Vector SHOULD be run under an unprivileged, deciated service account by
      default.
    * Vector's service account SHOULD NOT have the ability to overwrite Vector's
      binary or configuration files. The only directory the Vector service
      account should write to is Vector’s data directory.
* Network hardening
  * Configured sources and sinks SHOULD use encrypted channels by default.

## 7. Sizing & Scaling

[agent_architecture]: https://vector.dev/docs/setup/going-to-prod/arch/agent/
[aggregator_architecture]: https://vector.dev/docs/setup/going-to-prod/arch/aggregator/
[default_agent_configuration]: TODO...
[default_aggregator_configuration]: TODO...
[hardening]: https://vector.dev/docs/setup/going-to-prod/hardening/
[high_availability]: https://vector.dev/docs/setup/going-to-prod/high-availability/
[reference_architectures]: https://vector.dev/docs/setup/going-to-prod/arch/
[RFC 2119]: https://datatracker.ietf.org/doc/html/rfc2119
[terminology_document]: TODO...
[unified_architecture]: https://vector.dev/docs/setup/going-to-prod/arch/unified/
