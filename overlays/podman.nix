final: prev: {
  # nixpkgs 3160dc1172 (#569814) links podman against systemdLibs, whose libsystemd has no
  # compression support and so skips every zstd-compressed journal file: `podman logs` and
  # `podman events` on the journald driver come back empty, and distrobox-enter waits forever
  # for container_setup_done. Drop once nixpkgs' podman can read the journal again.
  podman =
    prev.lib.warnIfNot (builtins.any (p: p.drvPath == prev.systemdLibs.drvPath) prev.podman.buildInputs)
      "nixpkgs podman no longer links systemdLibs; drop overlays/podman.nix"
      (prev.podman.override { systemdLibs = final.systemd; });
}
