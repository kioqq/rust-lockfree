{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/packages/
  packages = [
      pkgs.curl
      pkgs.git
      pkgs.jq
      pkgs.just
      pkgs.tree

      ## rust
      pkgs.rustup
      pkgs.sccache
      pkgs.cargo-outdated
      pkgs.cargo-nextest
      pkgs.cargo-flamegraph
  ] ++ lib.optionals pkgs.stdenv.isDarwin (with pkgs.darwin.apple_sdk; [
       frameworks.SystemConfiguration
       frameworks.Security
       frameworks.CoreFoundation
     ]);

  # https://devenv.sh/languages/
  languages.nix.enable = true;


  languages.rust.enable = true;
  env.RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
  # languages.rust.mold.enable = true;

}