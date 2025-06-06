# This file is shell.nix in the tvix josh workspace,
# *and* used to provide the //tvix:shell attribute in a full depot checkout.
# Hence, it may not use depot as a toplevel argument.

{
  # This falls back to the tvix josh workspace-provided nixpkgs checkout.
  # In the case of depot, it's always set explicitly.
  pkgs ? (import ./nixpkgs {
    depotOverlays = false;
    depot.third_party.sources = import ./sources { };
  })
, ...
}:

pkgs.mkShell {
  name = "tvix-rust-dev-env";
  packages = [
    pkgs.cargo
    pkgs.cargo-machete
    pkgs.cargo-expand
    pkgs.clippy
    pkgs.hyperfine
    pkgs.nix_2_3 # b/313
    pkgs.pkg-config
    pkgs.rustc
    pkgs.rustfmt
    pkgs.protobuf
  ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
    pkgs.runc
  ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
    # We need these two dependencies in the ambient environment to be able to
    # `cargo build` on MacOS.
    pkgs.libiconv
    pkgs.buildPackages.darwin.apple_sdk.frameworks.Security
  ];

  # Set TVIX_BENCH_NIX_PATH to a somewhat pinned nixpkgs path.
  # This is for invoking `cargo bench` imperatively on the developer machine.
  # For tvix benchmarking across longer periods of time (by CI), we probably
  # should also benchmark with a more static nixpkgs checkout, so nixpkgs
  # refactorings are not observed as eval perf changes.
  shellHook = ''
    export TVIX_BUILD_SANDBOX_SHELL=${if pkgs.stdenv.isLinux then pkgs.busybox-sandbox-shell + "/bin/busybox" else "/bin/sh"}
    export TVIX_BENCH_NIX_PATH=nixpkgs=${pkgs.path}
  '';
}
