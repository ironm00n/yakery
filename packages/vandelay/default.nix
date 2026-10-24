{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  sqlite,
  cacert,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "vandelay";
  version = "1.0.12";

  src = fetchFromGitHub {
    owner = "stalwartlabs";
    repo = "vandelay";
    tag = "v${finalAttrs.version}";
    hash = "sha256-9bvheR4zLutIn8phClza8tpkE72L/hjZzroupnSk8ec=";
  };

  patches = [ ./01-use-url-origin.patch ];

  cargoHash = "sha256-w3Ux5EjpJlb1RnYz5IZVnagQ1lvIaNqnMuefEI02b/U=";

  # The only default feature, bundled-sqlite, vendors SQLite into rusqlite.
  buildNoDefaultFeatures = true;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ sqlite ];

  # The TLS connector tests load the system trust store.
  nativeCheckInputs = [ cacert ];

  meta = {
    description = "JMAP account importer-exporter and per-account backup tool";
    homepage = "https://github.com/stalwartlabs/vandelay";
    changelog = "https://github.com/stalwartlabs/vandelay/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = with lib.licenses; [
      asl20
      mit
    ];
    mainProgram = "vandelay";
  };
})
