{
  stdenvNoCC,
  buf,
  cacert,
  writableTmpDirAsHomeHook,

  version,
  src,
}:

# The BSR modules in proto/buf.lock are the only part of the build that needs
# the network, so only the descriptor image is fixed-output, never generated code.
stdenvNoCC.mkDerivation {
  pname = "zitadel-proto-image";
  inherit version src;

  nativeBuildInputs = [
    buf
    cacert
    writableTmpDirAsHomeHook
  ];

  buildPhase = ''
    runHook preBuild
    buf build proto --output zitadel.binpb
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm444 zitadel.binpb -t $out
    runHook postInstall
  '';

  dontFixup = true;

  outputHashMode = "recursive";
  outputHashAlgo = "sha256";
  outputHash = "sha256-w9DREhs8QPVn8komcUSy//XRajfGWYxKqgdcS7lmn9k=";
}
