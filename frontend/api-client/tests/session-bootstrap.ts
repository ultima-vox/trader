import { readFile } from "node:fs/promises";
import { VoxApiError, VoxClient } from "../src/client.ts";

const baseUrl = process.env.VOX_API_BASE_URL;
const bootstrapCredential = process.env.VOX_BOOTSTRAP_CREDENTIAL;
if (!baseUrl || !bootstrapCredential) throw new Error("test server configuration missing");

let cookie;
let mutationHadCsrf = false;
const browserFetch = async (input, init = {}) => {
  const headers = new Headers(init.headers);
  if (cookie && init.credentials !== "omit") headers.set("cookie", cookie);
  if (init.method === "DELETE") mutationHadCsrf = headers.has("x-vox-csrf");
  const response = await fetch(input, { ...init, headers });
  const setCookie = response.headers.get("set-cookie");
  if (setCookie) cookie = setCookie.split(";", 1)[0];
  return response;
};

const anonymous = await fetch(`${baseUrl}/api/v1/broker-connections`);
if (anonymous.status !== 401) throw new Error(`anonymous status ${anonymous.status}`);

const client = new VoxClient({ baseUrl, fetch: browserFetch });
const session = await client.postAuthSession({ bootstrap_credential: bootstrapCredential });
if (!session.csrf_token || !cookie?.startsWith("vox_session=")) {
  throw new Error("generated client did not establish browser session");
}

await client.brokerConnections();
const capabilities = await client.capabilities({});
if (!capabilities.unavailable.every((item) => item.owner)) {
  throw new Error("capability without owner");
}
const scopes = await client.runtimeScopes();
if (scopes.length !== 0) throw new Error("fresh runtime unexpectedly has scopes");

try {
  await client.deleteBrokerConnectionsConnection_id({
    connection_id: "connection:00000000-0000-4000-8000-000000000047",
  });
  throw new Error("unknown connection delete unexpectedly succeeded");
} catch (error) {
  if (!(error instanceof VoxApiError) || error.status !== 404) throw error;
}
if (!mutationHadCsrf) throw new Error("generated client omitted CSRF header on mutation");

const served = await (await browserFetch(`${baseUrl}/api/v1/openapi.json`, {
  method: "GET",
  credentials: "same-origin",
})).json();
const committed = JSON.parse(
  await readFile(new URL("../../../docs/api/openapi.json", import.meta.url), "utf8"),
);
if (JSON.stringify(served) !== JSON.stringify(committed)) {
  throw new Error("served OpenAPI differs from committed artifact");
}

console.log("generated client: session cookie + automatic CSRF verified");
