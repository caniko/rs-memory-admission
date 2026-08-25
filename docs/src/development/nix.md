# Nix

The flake builds the crate with crane through `git+https://github.com/caniko/harbor-rs.git`.

```sh
nix build
nix flake check
nix develop
```

The website and documentation are available as separate package outputs:

```sh
nix build .#website
nix build .#docs
nix build .#site
```

The combined `site` output serves the Zola site at the root and mdBook documentation under `/docs/`.

## Release Publishing

The tag-triggered publish workflow runs from `.forgejo/workflows/publish-crate.yaml`.
It expects the Codeberg repository secret `CRATES_IO_API_TOKEN` and exports it
as Cargo's runtime `CARGO_REGISTRY_TOKEN` before running `cargo publish`.

Do not add a `cargo login` step to this workflow. Modern Cargo reads registry
tokens from stdin for `cargo login`, while `cargo publish` already consumes
`CARGO_REGISTRY_TOKEN` directly.
