import { mkdir, readFile, rename, rmdir } from "node:fs/promises";
import { VoxApiError, VoxClient } from "../src/client.ts";

const baseUrl = process.env.VOX_API_BASE_URL;
const bootstrapCredential = process.env.VOX_BOOTSTRAP_CREDENTIAL;
const platformDatabase = process.env.VOX_PLATFORM_DB;
if (!baseUrl || !bootstrapCredential || !platformDatabase) {
  throw new Error("test server configuration missing");
}

const expectApiError = async (promise, status, category, retryable) => {
  try {
    await promise;
    throw new Error(`request unexpectedly succeeded; expected ${status}`);
  } catch (error) {
    if (!(error instanceof VoxApiError)) throw error;
    if (error.status !== status) throw new Error(`status ${error.status}; expected ${status}`);
    const body = error.body;
    if (
      body.category !== category ||
      body.retryable !== retryable ||
      typeof body.code !== "string" ||
      body.code.length === 0 ||
      typeof body.message !== "string" ||
      body.message.length === 0 ||
      typeof body.correlation_id !== "string" ||
      body.correlation_id.length === 0
    ) {
      throw new Error(`invalid canonical ApiError: ${JSON.stringify(body)}`);
    }
  }
};

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

const anonymousClient = new VoxClient({ baseUrl });
await expectApiError(
  anonymousClient.brokerConnections(),
  401,
  "AUTHENTICATION",
  false,
);

const client = new VoxClient({ baseUrl, fetch: browserFetch });
const session = await client.postAuthSession({ bootstrap_credential: bootstrapCredential });
if (!session.csrf_token || !cookie?.startsWith("vox_session=")) {
  throw new Error("generated client did not establish browser session");
}

const noCsrfClient = new VoxClient({ baseUrl, fetch: browserFetch });
await expectApiError(
  noCsrfClient.deleteBrokerConnectionsConnection_id({
    connection_id: "connection:00000000-0000-4000-8000-000000000047",
  }),
  403,
  "PERMISSION",
  false,
);

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

const unavailableBackup = `${platformDatabase}.auth-error-test`;
await rename(platformDatabase, unavailableBackup);
await mkdir(platformDatabase);
try {
  await expectApiError(client.brokerConnections(), 503, "TRANSIENT", true);
} finally {
  await rmdir(platformDatabase);
  await rename(unavailableBackup, platformDatabase);
}

console.log("generated client: session/CSRF + canonical 401/403/503 verified");
