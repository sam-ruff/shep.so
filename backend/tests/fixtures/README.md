# Synthetic signature and HTTPS fixtures

The `oidc-test-*` keys sign synthetic Google claims. The `https-test-*` certificate/key serves the isolated loopback Playwright proxy. Both sets are public test material, do not identify a Google project/user, and must never be used for deployed credentials or TLS. The browser harness alone accepts the self-signed test certificate; production requires valid public HTTPS and real Google verification.
