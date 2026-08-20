{
  description = "Memory-aware admission gate for Rust work pipelines";

  inputs = {
    rs-harbor.url = "git+https://github.com/caniko/rs-harbor.git?ref=trunk&rev=05cc4f162b55fa904b687db1821e2463fa813e50";

    nixpkgs.follows = "rs-harbor/nixpkgs";
    rust-overlay.follows = "rs-harbor/rust-overlay";
    crane.follows = "rs-harbor/crane";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    plinth = {
      url = "git+https://codeberg.org/caniko/plinth.git";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rs-harbor,
    plinth,
    flake-utils,
    rust-overlay,
    treefmt-nix,
    git-hooks,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      lib = pkgs.lib;
      toolchain = rs-harbor.lib.mkToolchain {inherit pkgs; toolchainProfile = "nightly";};
      cross = rs-harbor.lib.mkCross {inherit pkgs system;};
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

        docs = rs-harbor.lib.mkDocsShell {
          inherit pkgs cross;
          inherit (toolchain) craneLib;
          checks = self.checks.${system};
          packages = with pkgs; [
            mdbook
            plinth.packages.${system}.plinth-project
            pre-commit
            rust-analyzer
          ] ++ pre-commit-check.enabledPackages;
          extraShellHook = ''
            ${pre-commit-check.shellHook}
            echo "Project site: plinth-project serve --config website/plinth-project.toml"
            echo "Documentation: mdbook serve docs"
          '';
        };
      };
    });
}
