# Nix

The flake builds the crate with crane through `git+https://codeberg.org/caniko/rs-harbor.git`.

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
