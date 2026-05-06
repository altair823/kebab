# Claude Code integration

Skill packages that let [Claude Code](https://claude.ai/claude-code) call `kebab` automatically when a question would benefit from the user's local KB.

## Available skills

| Skill | Trigger | What it does |
|-------|---------|--------------|
| [`kebab`](kebab/SKILL.md) | Internal / org-specific questions, runbooks, indexed-doc lookups | Calls `kebab search --json` / `kebab ask --json` and folds the results into the answer with citations |

## Install

User-level (every Claude Code session on this machine):

```bash
# from a kebab repo checkout
cp -r integrations/claude-code/kebab ~/.claude/skills/

# verify
ls ~/.claude/skills/kebab/SKILL.md
```

Or symlink so `git pull` in the repo updates the skill in place:

```bash
mkdir -p ~/.claude/skills
ln -s "$(pwd)/integrations/claude-code/kebab" ~/.claude/skills/kebab
```

Project-level (only loads when Claude Code runs in a specific project):

```bash
mkdir -p <project>/.claude/skills
cp -r integrations/claude-code/kebab <project>/.claude/skills/
```

After install, start a fresh Claude Code session — the skill self-registers from its frontmatter `description` and is invoked automatically when a matching question shows up. No config edit needed.

## Customization

The shipped `SKILL.md` is generic on purpose — it triggers on any "internal / org-specific" cue. To make Claude Code more eager (or less) for **your** corpus, edit the frontmatter `description` of your local copy and add the team / system / acronym keywords that should trigger the skill (e.g. `MLOps`, `DMQ`, `AiSuite`). Don't PR those keywords back into this repo — they're per-user.

A symlink install + a `~/.claude/skills/kebab/SKILL.md.local` patch script is one pattern; another is to keep a fork branch with personalized frontmatter and rebase on `main`.

## Update policy

The skill consumes `kebab`'s wire schema v1 (`schema_version` fields like `search_hit.v1`, `answer.v1`). When the wire schema major-bumps to v2, this skill is updated in the same PR — see the project root [`CLAUDE.md`](../../CLAUDE.md#wire-schema-v1) §Wire schema v1.

## Other hosts

`kebab` exposes the same `--json` contract to any agent host. To add a new integration:

1. Drop a directory under `integrations/<host>/` mirroring the structure here.
2. Reference `docs/wire-schema/v1/` for the JSON shapes.
3. Link from this `README.md` table.

A native MCP server (`kebab serve --mcp`) and an HTTP wrapper are listed in the root [README §외부 AI 통합](../../README.md#외부-ai-통합) as future options.
