{ lib }:
lib.listToAttrs (
  lib.imap1
    (i: name: {
      inherit name;
      value = {
        uid = 2000 + i;
        homeQuotaGiB = 32;
      };
    })
    [
      "alpha"
      "beta"
      "gamma"
      "delta"
      "epsilon"
      "zeta"
      "eta"
      "theta"
      "iota"
      "kappa"
      "lambda"
      "mu"
      "nu"
      "xi"
      "omicron"
      "pi"
      "rho"
      "sigma"
      "tau"
      "upsilon"
      "phi"
      "chi"
      "psi"
      "omega"
    ]
)
