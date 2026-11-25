import postgres from "npm:postgres@3.4.7";
import { createStats } from "./handler.ts";
import { siteStats } from "./ga4.ts";

// Read-only, service-side connection; the stats page never sees database access.
const db = postgres(Deno.env.get("SUPABASE_DB_URL")!, {
  max: 1,
  prepare: false,
});

Deno.serve(
  createStats({
    env: (name) => Deno.env.get(name),
    fetch,
    sql: (query) => db.begin("read only", (tx) => tx.unsafe(query)),
    site: siteStats,
  }),
);
