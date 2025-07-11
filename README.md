# NixOS Config

My NixOS configuration.

## First install

this flake uses experimental nix features!

Either manually add `pipe-operator` (for lix) or `pipe-operators` (for nix)
to your `~/.config/nix/nix.conf` config:

```
experimental-features = nix-command build pipe-operator
```

Alternatively, you can build the derivation and switch to it manually, and
proceed to run `nixos-rebuild` as normal:

```sh
host=$(cat /etc/hostname); \
ref="git+file://$(pwd)#nixosConfigurations.\"$host\".config.system.build.toplevel"; \
out=$(nix --extra-experimental-features 'nix-command flakes pipe-operator' build \
    --print-out-paths $ref --keep-going --show-trace --impure --no-link | tail -n1); \
output="$out" | tail -n1; \
sudo "$out/bin/switch-to-configuration" switch
```

> [!NOTE]
> The above script doesn't update the bootloader, run `nixos-rebuild` afterwards.
