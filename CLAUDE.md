On start, read files in [llm-docs](./llm-docs).

Reference file [LLM-TODO.md](./llm-docs/LLM-TODO.md) for current project TODO state.

### llm-docs
These apply __ONLY__ to editing llm-docs:

- Reference and update files in directory [llm-docs](./llm-docs/) with _relevant_ knowledge required for your successors to understand/improve and complete the goals in LLM-TODO.md.
- Above all else, prioritize succinctness in what you write in llm-docs - do not waste context windows of your successors on anything irrelevant.
- The app spec consists of the compound summary of human facing (README.md and example config) and LLM facing documentation (LLM-TODO.md, llm-docs and README.md). This spec must be kept in sync with the project state. When the project has deviated from the spec, notify the user, but do not attempt to sync the project with the spec without user approval. 

### Source references

- When looking up sources from other projects, just clone them into `local` directory

### Env

- You are running in a sandboxed environment, your options for live testing changes are limited. Especially given the fact that the project in question relies on user interaction.

### README

- When editing README.md, do not overwrite sections written by author. Write in your own section: `Machine summary`
- If there were changes to configuration format, update README and the example config.

### Git commits

- Make a git commit after a shippable chunk of work are done
- Add tests for each new shippable chunk of work (or update existing tests to cover the change)
- Before commit, update llm-docs and LLM-TODO, and if there are new features or user-facing changes, README.md. Guidelines in [llm-docs](#llm-docs) section apply.
- **Run tests before commit**: `cargo test` and `nix run .#test`
- Test both Nix and Cargo builds before commit: `cargo build --release` and `nix build`
- Do not amend commits, make new commits for your changes
- Do not attribute commits to the LLM
