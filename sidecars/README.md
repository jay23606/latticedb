# sidecars

Webhook receivers, importers and delivery workers. These are the only components allowed to make
outbound connections (ADR 0003). Must not: run inside the core.
