---
name: aws-project-migration
description: Inspect, migrate, deploy, and maintain small applications on AWS through a constrained deployment role. Use when planning a migration, checking existing infrastructure, preparing a release, reviewing service costs, or investigating a failed deployment. Before changing infrastructure, identify the application, its current environment, its deployment configuration, and the permissions available to the operator. Verify the running service after a deployment and preserve enough evidence to explain the result. Keep credentials in the configured secret store and distinguish a completed build from a verified deployment.
license: MIT
compatibility: Requires AWS CLI and a configured deployment role.
metadata:
  category: infrastructure
  audience: technical operators
---
# AWS project migration

Inspect the application and its current infrastructure before choosing a deployment approach. Record the runtime, storage requirements, background jobs, and public entry points so the migration preserves the behavior people depend on.

## Inspect the existing deployment

Read the deployment configuration and compare it with the resources that currently exist. Confirm the selected account and region before making a change. Check which services own persistent data and how they are backed up.

Review access through the constrained deployment role. Request only the permissions needed for the planned operation, and keep credentials in the configured secret store.

## Prepare and verify

Build the application, run its checks, and inspect the resulting package. Deploy to the selected environment, then verify the actual request path through the application and its dependencies.

Report the changed resources, the verification result, and any acceptance checks that still need to happen. Preserve the previous deployment until the replacement has passed verification.
