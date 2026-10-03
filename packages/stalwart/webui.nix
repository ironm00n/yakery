{
  buildNpmPackage,
  stalwart,
  fetchFromGitHub,
  nix-update-script,
  zip,
  oauthClientId ? "stalwart-webui",
  lib,
}:

buildNpmPackage (finalAttrs: {
  pname = "webui";
  version = "1.0.11";

  src = fetchFromGitHub {
    owner = "stalwartlabs";
    repo = "webui";
    tag = "v${finalAttrs.version}";
    hash = "sha256-luaNHpPg7XHnPfQqhLEq+wW1/crXLogzyyltFr4mdBk=";
  };

  npmDepsHash = "sha256-qe9cSrvs6kWwgbOO0xL7MBaJvICOvyuLFVi9R0dgnXQ=";

  patches = [ ./01-oauth-prompt.patch ];

  nativeBuildInputs = [ zip ];

  preBuild = ''
    cat > .env.production <<EOF
    VITE_OAUTH_CLIENT_ID=${oauthClientId}
    EOF
  '';

  installPhase = ''
    runHook preInstall
    cd dist
    zip -r "$out" .
    runHook postInstall
  '';

  passthru = {
    updateScript = nix-update-script { };
  };

  meta = {
    description = "Web administration module for the Stalwart server";
    homepage = "https://github.com/stalwartlabs/webui";
    changelog = "https://github.com/stalwartlabs/webui/blob/${finalAttrs.src.tag}/CHANGELOG.md";
    inherit (stalwart.meta) license maintainers;
  };
})
