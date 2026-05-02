---
title: Python Snippets
tags: [python, language]
lang: en
created_at: 2024-05-01T00:00:00Z
updated_at: 2024-05-02T00:00:00Z
source_type: note
trust_level: primary
---

# Python Snippets

Quick reference for everyday Python tasks.

## List comprehensions

Filter and transform in one pass: `[x*2 for x in xs if x > 0]`. Cleaner than
the map+filter pair when the predicate is simple.

## Decorators

Wrap a function in another function. `functools.wraps` preserves the
docstring and `__name__` of the inner function on the outer wrapper.
