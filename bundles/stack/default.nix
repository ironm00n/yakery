{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mapAttrs'
    mapAttrsToList
    mkEnableOption
    mkIf
    mkOption
    nameValuePair
    types
    ;
  cfg = config.bundles.stack;

  shim =
    pkgs':
    pkgs'.runCommandCC "thread-stack.so" { } ''
      $CC -O2 -shared -fPIC -DTHREAD_STACK_SIZE=${toString cfg.threadStackSize} -o $out ${./thread-stack.c}
    '';

  # ld.so's $PLATFORM: AT_PLATFORM, which glibc renames on Intel CPUs (sysdeps/x86/cpu-features.c)
  shims =
    let
      x64 = shim pkgs;
      x86 = shim pkgs.pkgsi686Linux;
    in
    pkgs.linkFarm "thread-stack" (
      mapAttrs' (platform: nameValuePair "${platform}/thread-stack.so") {
        x86_64 = x64;
        haswell = x64;
        xeon_phi = x64;
        i686 = x86;
        i586 = x86;
      }
    );
in
{
  options.bundles.stack = {
    enable = mkEnableOption "an unlimited soft stack limit for login sessions";

    threadStackSize = mkOption {
      type = types.ints.positive;
      default = 8 * 1024 * 1024;
      description = "Default pthread stack size in bytes under an unlimited stack limit, where glibc would fall back to 2 MiB.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isx86_64;
        message = "bundles.stack only knows glibc's $PLATFORM names for x86_64 hosts";
      }
    ];

    # root's nix builds in-process rather than through the daemon, so its builders inherit this
    security.pam.loginLimits =
      mapAttrsToList
        (domain: value: {
          inherit domain value;
          type = "soft";
          item = "stack";
        })
        {
          "*" = "unlimited";
          root = "8192";
        };

    environment.etc."ld-nix.so.preload".text = ''
      ${shims}/$PLATFORM/thread-stack.so
    '';
  };
}
