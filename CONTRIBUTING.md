# Contributing

Thanks for helping. The rules every change follows (people and AI agents alike) are in
[AGENTS.md](AGENTS.md); mistakes already made are in [docs/pitfalls.md](docs/pitfalls.md). In short:

- **Pick work from [docs/ROADMAP.md](docs/ROADMAP.md)** and claim it first: add
  `— claimed: <your name>, branch <branch>` to its line and open a draft PR.
- **One change per PR**, on a branch named `feature/`, `bugfix/`, `docs/`, `test/`, `refactor/` or
  `chore/` plus the roadmap id. PRs are squash-merged, so the title is the commit message.
- **Clean room**: behaviour comes from observation and public documentation only.
- **Never crash, safe Rust only, no secrets or personal data anywhere.**

Before you ask for review: `cargo xtask ci`.

Code is dual-licensed under MIT or Apache-2.0 ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)); by contributing you agree your work is licensed the same way.
