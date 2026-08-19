## Summary

-

## Checklist

- [ ] Version stays on **0.1.x** (do not introduce 0.2 / 1.0)
- [ ] Do **not** auto-merge release-please PRs

## Skip co-authors

Local commits get these trailers from `.githooks/prepare-commit-msg` (enable with `./scripts/install-hooks.sh`):

```
Co-authored-by: Duyet Le <me@duyet.net>
Co-authored-by: duyetbot <bot@duyet.net>
```

To skip one commit: `ANYR_SKIP_COAUTHORS=1 git commit`

- [ ] I skipped co-author trailers on purpose
