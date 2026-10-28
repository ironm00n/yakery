# A wait's periods in seconds: how long it waits for a push before scanning anyway, its first pause before signing in
# again, and how long it goes without a finished scan before failing. The VM test builds with shorter ones.
{
  recheck = 150;
  backoff = 5;
  stale = 300;
}
