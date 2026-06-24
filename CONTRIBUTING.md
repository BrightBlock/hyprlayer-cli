# Contributing to Hyprlayer

Thanks for your interest in contributing! This guide covers the one required
legal step (signing the CLA) and how to get a change merged.

## Signing the CLA

Hyprlayer requires every contributor to agree to a **Contributor License
Agreement** before their first contribution can be merged. Under the CLA you
**assign the copyright in your contributions to BrightBlock Labs** (with a broad
exclusive-license fallback for jurisdictions where assignment isn't possible),
so that BrightBlock Labs can maintain, relicense, and defend the project as a single
copyright holder. You keep a full license to use your own contributions for any
other purpose — see Section 4 of the agreement.

Read the agreement first: **[CLA.md](CLA.md)**.

### Sign in the pull request

You don't need to do anything before opening a PR. When you open your **first**
pull request, the CLA Assistant bot checks whether you've signed. If you
haven't, it posts a comment with a link to [CLA.md](CLA.md) and asks you to reply
with a single line:

```
I have read the CLA Document and I hereby sign the CLA
```

Posting that comment is your electronic signature and covers all of your current
and future contributions — you only sign once. The bot then turns the PR's CLA
status green and merging is unblocked. If the check ever gets out of sync, comment
`recheck` to re-run it.

### Contributing on behalf of a company

The same signature covers company contributions. If you're contributing as part
of your job, sign in the PR exactly as above — by doing so you represent that you
have authority to bind your employer (see Section 6.1 of the CLA). It helps to
mention the company in your PR (for example, "Signing on behalf of Acme Corp")
for the record, though that isn't required.

## Development setup

Hyprlayer is a Rust (edition 2024) CLI built with Cargo.

```bash
# Build
cargo build

# Run the test suite
cargo test

# Format and lint before opening a PR
cargo fmt --all
cargo clippy --all-targets
```

## Opening a pull request

1. Fork the repository and create a topic branch off `master`.
2. Make your change. Keep it focused — one logical change per PR is easiest to
   review.
3. Add or update tests where it makes sense, and make sure `cargo test` passes.
4. Run `cargo fmt --all` and address `cargo clippy` warnings.
5. Open the PR against `master`, describe what changed and why, and link any
   related issue.
6. Sign the CLA when the bot asks (see above), then a maintainer will review.

## Reporting bugs and requesting features

Open a [GitHub issue](https://github.com/BrightBlock/hyprlayer-cli/issues).
For bugs, include your OS, the `hyprlayer` version (`hyprlayer --version`), the
command you ran, and what you expected versus what happened.

## Code of conduct

Be respectful and constructive. Harassment, personal attacks, and other abusive
behavior aren't welcome here. Maintainers may remove comments, commits, or
contributions that don't meet this standard.
