{
  enable = true;
  # see about:policies#documentation
  policies = {
    DisablePocket = true;
    DisableTelemetry = true;
    FirefoxSuggest = {
      "WebSuggestions" = false;
      "SponsoredSuggestions" = false;
      "ImproveSuggest" = false;
      "Locked" = false;
    };
    FirefoxHome = {
      "Search" = true;
      "TopSites" = false;
      "SponsoredTopSites" = false;
      "Highlights" = false;
      "Pocket" = false;
      "SponsoredPocket" = false;
      "Snippets" = false;
      "Locked" = false;
    };
  };
}
