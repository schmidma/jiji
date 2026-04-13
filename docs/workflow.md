# Development Workflow

Use a linear history with concise atomic commits.

## Rules

1. Branch new work from `main`.
2. Local WIP commits are allowed on unpublished branches.
3. Before integrating a branch, rewrite it on top of `origin/main` into the smallest correct stack of semantic atomic commits.
4. Prefer one commit for small changes and at most `2-3` commits when there are real logical boundaries.
5. Update feature branches with `git rebase main` or `git rebase origin/main`.
6. Do not merge `main` into feature branches.
7. Integrate finished work into `main` only by fast-forward.
8. Do not create merge commits as part of the normal workflow.
9. Do not rewrite published history as part of normal workflow.
10. Do not force-push `main`.

## Recommended Local Git Config

These settings help enforce it locally:

```bash
git config pull.rebase true
git config rebase.autoStash true
git config merge.ff only
git config fetch.prune true
```

These settings are recommendations only. This repository does not apply them automatically.

## Typical Flow

1. Update `main`.
2. Create a feature branch.
3. Make changes and verify locally.
4. Fetch latest refs.
5. Rewrite the unpublished branch on top of `origin/main` into one or a very small stack of semantic atomic commits.
6. Verify again after the rewrite.
7. Fast-forward `main` to the verified branch tip.
8. Publish the rewritten history.
9. Delete the merged branch and remove its worktree.

## Current Scope

This policy is local-first for now.

- No GitHub branch protection yet
- No required PR workflow yet
- No helper scripts or auto-installed hooks

If the project grows beyond one active contributor, stronger hosted enforcement can be added later.
