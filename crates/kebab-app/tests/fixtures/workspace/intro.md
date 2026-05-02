---
title: Introduction to Rust
tags: [rust, language]
lang: en
created_at: 2024-03-01T00:00:00Z
updated_at: 2024-03-02T00:00:00Z
source_type: note
trust_level: primary
---

# Introduction to Rust

Rust is a systems programming language focused on safety, speed, and concurrency.
The compiler enforces memory safety without a garbage collector.

## Ownership

Each value has a single owner. When the owner goes out of scope the value is
dropped. References are borrows that the compiler tracks at compile time.

## Concurrency

Threads in Rust use the ownership system to prevent data races. The Send and
Sync traits codify which types can move between threads.
