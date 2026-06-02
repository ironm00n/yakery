{
  config,
  lib,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkOption
    mkIf
    types
    ;
  cfg = config.bundles.distributed-builds;

  # Reached over Netbird; bare names resolve via the overlay's DNS search
  # domain (pushed system-wide, so the root daemon sees it too). publicKey is
  # each builder's /etc/ssh host key, pinned so first contact isn't TOFU.
  builders = [
    {
      host = "desktop";
      system = "x86_64-linux";
      maxJobs = 8;
      speedFactor = 4;
      features = [ "nixos-test" "benchmark" "big-parallel" "kvm" ];
      publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGvouuDpNF1MtGJSyIcHzScPkcNtoOMDxbPqX1GuxkTy";
    }
    {
      host = "oracle-a1-flex-1";
      system = "aarch64-linux";
      maxJobs = 2;
      speedFactor = 1;
      features = [ "big-parallel" ];
      publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHjo9Px5k7d3XR607qNN0++5MSTHiVg/XYwL+DI6YaRW";
    }
    {
      host = "oracle-a1-flex-2";
      system = "aarch64-linux";
      maxJobs = 2;
      speedFactor = 3;
      features = [ "big-parallel" ];
      publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICmkMGbICuPU35PNDjKWMVfFg6lQPzUyZWxCG0ubhPiu";
    }
    {
      host = "oracle-a1-flex-3";
      system = "aarch64-linux";
      maxJobs = 2;
      speedFactor = 1;
      features = [ "big-parallel" ];
      publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIINhu9YoVtXVSI1Ew/ZJkzbaP/zFpPSjUf8Le4p//h5c";
    }
  ];

  active = builtins.filter (b: b.host != config.host.hostname) builders;
in
{
  options.bundles.distributed-builds = {
    enable = mkEnableOption "offloading Nix builds to the desktop + a1-flex farm over Netbird";

    sshKey = mkOption {
      type = types.str;
      default = "/home/ironmoon/.ssh/id_ed25519";
      description = "Private key the root Nix daemon uses to reach builders.";
    };
  };

  config = mkIf cfg.enable {
    nix.distributedBuilds = true;
    nix.settings.builders-use-substitutes = true;

    nix.buildMachines = map (b: {
      hostName = b.host;
      sshUser = "root";
      sshKey = cfg.sshKey;
      protocol = "ssh-ng";
      systems = [ b.system ];
      inherit (b) maxJobs speedFactor;
      supportedFeatures = b.features;
    }) active;

    programs.ssh.knownHosts = builtins.listToAttrs (
      map (b: {
        name = b.host;
        value.publicKey = b.publicKey;
      }) active
    );
  };
}
