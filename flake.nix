{
  description = "Memory-aware admission gate for Rust work pipelines";

  inputs = {
    harbor.url = "git+https://github.com/caniko/harbor.git?ref=feat/harbor-monorepo-components&rev=e8b1996580e4ecd51dd402edf4a16d9df88e3a1b";

    nixpkgs.follows = "harbor/nixpkgs";
    rust-overlay.follows = "harbor/rust-overlay";
    crane.follows = "harbor/crane";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "harbor/nixpkgs-darwin";
    };
    plinth = {
      url = "git+https://github.com/caniko/plinth.git?ref=migrate/harbor-combined-20261012-features&rev=534dfa6dd6217db97b7ccd313501120a3c34d912";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    harbor,
    plinth,
    flake-utils,
    rust-overlay,
    treefmt-nix,
    git-hooks,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs =
        import (
          if system == "x86_64-darwin"
          then harbor.inputs.nixpkgs-darwin
          else nixpkgs
        ) {
          inherit system;
          overlays = [(import rust-overlay)];
        };
      lib = pkgs.lib;
      toolchain = harbor.lib.rust.mkToolchain {
        inherit pkgs;
        toolchainProfile = "nightly";
      };
      cross = harbor.lib.rust.mkCross {inherit pkgs system;};
      inherit (toolchain) craneLib;

      src = craneLib.cleanCargoSource ./.;

      commonArgs = {
        inherit src;
        strictDeps = true;
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      package = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
        });

      docs = pkgs.stdenv.mkDerivation {
        pname = "memory-admission-docs";
        version = "0.1.0";
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.maybeMissing ./docs;
        };
        nativeBuildInputs = [pkgs.mdbook];
        phases = ["buildPhase" "installPhase"];
        buildPhase = ''
          cp -r --no-preserve=mode $src/docs docs
          mdbook build docs
        '';
        installPhase = ''
          cp -r docs/book $out
        '';
      };

      site = plinth.lib.${system}.mkProjectSite {
        pname = "memory-admission-website";
        domain = "memory-admission.tartanoglu.com";
        configPath = ./website/plinth-project.toml;
        docsPackage = docs;
      };

      treefmtEval = treefmt-nix.lib.evalModule pkgs (import ./nix/treefmt.nix);
      pre-commit-check = git-hooks.lib.${system}.run {
        src = ./.;
        hooks = import ./nix/pre-commit.nix {
          inherit pkgs;
          treefmtWrapper = treefmtEval.config.build.wrapper;
          rustToolchain = toolchain.rustToolchain;
        };
      };
    in {
      packages = {
        default = package;
        website = site;
        inherit docs site;
      };

      apps.deploy-pages = plinth.lib.${system}.mkDeployPagesApp {
        domain = "memory-admission.tartanoglu.com";
      };

      formatter = treefmtEval.config.build.wrapper;

      checks = {
        default = package;
        formatting = treefmtEval.config.build.check self;

        clippy = craneLib.cargoClippy (commonArgs
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
          });

        fmt = craneLib.cargoFmt {
          inherit src;
        };
        # Fail if flake inputs ever point at the retired Codeberg/Codefloe
        # mirrors again (fleet migrated to github.com/caniko/*).
        # sourceUrl package metadata is excluded: informational only, not fetched.
        host-pinning = let
          # Split across literals so this file never matches its own pattern.
          staleHosts = "cod" + "eberg|cod" + "efloe";
        in
          pkgs.runCommand "rs-memory-admission-host-pinning" {} ''
            if ${pkgs.lib.getExe pkgs.ripgrep} -v "sourceUrl" ${./flake.nix} ${./flake.lock} \
              | ${pkgs.lib.getExe pkgs.ripgrep} -q "${staleHosts}"; then
              echo "ERROR: retired forge host in flake inputs:" >&2
              ${pkgs.lib.getExe pkgs.ripgrep} -v "sourceUrl" ${./flake.nix} ${./flake.lock} \
                | ${pkgs.lib.getExe pkgs.ripgrep} -n "${staleHosts}" >&2 || true
              exit 1
            fi
            touch $out
          '';
      };

      devShells = {
        default = craneLib.devShell {
          checks = self.checks.${system};
          packages = with pkgs;
            [
              alejandra
              cargo-nextest
              mdbook
              pre-commit
              prettier
              rust-analyzer
              taplo
            ]
            ++ pre-commit-check.enabledPackages;
          shellHook = ''
            ${pre-commit-check.shellHook}
            echo "Documentation: cd docs && mdbook serve"
          '';
        };

        docs = harbor.lib.rust.mkDocsShell {
          inherit pkgs cross;
          inherit (toolchain) craneLib;
          checks = self.checks.${system};
          packages = with pkgs;
            [
              mdbook
              plinth.packages.${system}.plinth-project
              pre-commit
              rust-analyzer
            ]
            ++ pre-commit-check.enabledPackages;
          extraShellHook = ''
            ${pre-commit-check.shellHook}
            echo "Project site: plinth-project serve --config website/plinth-project.toml"
            echo "Documentation: mdbook serve docs"
          '';
        };
      };
    });
}
