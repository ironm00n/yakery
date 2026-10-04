# Stand-in for the storage box: a plain sshd with user `u`, pinned by a throwaway host key.
{ pkgs }:
rec {
  hostKey = {
    public = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPu6ruqbYgSS7vEPjQxWDL21tKfI6wS7uGT5IrLeq5Pi test-box-host";
    private = ''
      -----BEGIN OPENSSH PRIVATE KEY-----
      b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
      QyNTUxOQAAACD7uq7qm2IEku7xD40MVgy9tbSnyOsEu7hk+SKy3quT4gAAAJB3Q8s6d0PL
      OgAAAAtzc2gtZWQyNTUxOQAAACD7uq7qm2IEku7xD40MVgy9tbSnyOsEu7hk+SKy3quT4g
      AAAEBdFRb1+jk8oNJhvNPOnStqRxxVZPaUciSq1mcLDWuI1fu6ruqbYgSS7vEPjQxWDL21
      tKfI6wS7uGT5IrLeq5PiAAAADXRlc3QtYm94LWhvc3Q=
      -----END OPENSSH PRIVATE KEY-----
    '';
  };

  node = {
    services.openssh.enable = true;
    services.openssh.hostKeys = [ ];
    environment.etc."ssh/ssh_host_ed25519_key" = {
      text = hostKey.private;
      mode = "0600";
    };
    environment.etc."ssh/ssh_host_ed25519_key.pub".text = hostKey.public;
    users.users.u.isNormalUser = true;
  };
}
