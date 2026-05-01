---
title: Cargo Notes
tags: [rust, cargo, tools]
lang: en
created_at: 2024-04-01T00:00:00Z
updated_at: 2024-04-02T00:00:00Z
source_type: note
trust_level: primary
---

# Cargo Notes

Cargo is the Rust package manager and build tool.

## Workspaces

A workspace is a set of packages that share the same `Cargo.lock` and output
directory. Member crates are listed under `[workspace.members]`.

## Features

Cargo features let crates expose optional functionality behind a feature flag.
Default features are enabled unless `default-features = false` is set.
