// A stand-in for Google as far as signing in to Drive goes. Its consent screen sends the browser straight back with a
// code, its token endpoint checks the PKCE verifier, and Drive's about.get says who signed in. The e2e server's
// googleUrl points at it, and server/drive/auth.test.ts runs it on a port of its own.
//
//   node e2e/fake-google.ts [port]
import { createHash, randomBytes } from "node:crypto";
import * as http from "node:http";

export const FAKE_GOOGLE_PORT = 4397;
export const FAKE_GOOGLE_CLIENT = { id: "fake-google-client", secret: "fake-google-secret" };
export const FAKE_GOOGLE_USER = { permissionId: "07362580457318461234", emailAddress: "jane@drive.example", displayName: "Jane Doe" };
const SCOPE = "https://www.googleapis.com/auth/drive.file";

/** Google's endpoints at their usual paths; calls lists each request as "METHOD /path". */
export function fakeGoogle() {
  const calls: string[] = [];
  const codes = new Map<string, { challenge: string; redirectUri: string }>();
  const refreshTokens = new Set<string>();
  let issued = 0;
  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const route = `${req.method} ${url.pathname}`;
    calls.push(route);
    const answer = (status: number, body: unknown) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    const tokens = () => {
      issued += 1;
      return { access_token: `fake-access-${issued}`, expires_in: 3600, scope: SCOPE, token_type: "Bearer" };
    };
    let body = "";
    req.on("data", (chunk: Buffer) => (body += chunk));
    req.on("end", () => {
      const form = new URLSearchParams(body);
      switch (route) {
        case "GET /o/oauth2/v2/auth": {
          const query = url.searchParams;
          if (query.get("client_id") !== FAKE_GOOGLE_CLIENT.id || query.get("code_challenge_method") !== "S256") return answer(400, { error: "invalid_request" });
          const code = randomBytes(8).toString("hex");
          codes.set(code, { challenge: query.get("code_challenge") ?? "", redirectUri: query.get("redirect_uri") ?? "" });
          const back = new URL(query.get("redirect_uri") ?? "");
          back.search = new URLSearchParams({ code, state: query.get("state") ?? "", scope: query.get("scope") ?? "" }).toString();
          return res.writeHead(302, { location: back.toString() }).end();
        }
        case "POST /token": {
          if (form.get("client_id") !== FAKE_GOOGLE_CLIENT.id || form.get("client_secret") !== FAKE_GOOGLE_CLIENT.secret) {
            return answer(401, { error: "invalid_client" });
          }
          if (form.get("grant_type") === "refresh_token") {
            return refreshTokens.has(form.get("refresh_token") ?? "") ? answer(200, tokens()) : answer(400, { error: "invalid_grant" });
          }
          const asked = codes.get(form.get("code") ?? "");
          codes.delete(form.get("code") ?? "");
          const verifier = form.get("code_verifier") ?? "";
          if (!asked || asked.redirectUri !== form.get("redirect_uri") || createHash("sha256").update(verifier).digest("base64url") !== asked.challenge) {
            return answer(400, { error: "invalid_grant" });
          }
          const refreshToken = `fake-refresh-${randomBytes(4).toString("hex")}`;
          refreshTokens.add(refreshToken);
          return answer(200, { ...tokens(), refresh_token: refreshToken });
        }
        case "POST /revoke":
          refreshTokens.delete(form.get("token") ?? "");
          return answer(200, {});
        case "GET /drive/v3/about":
          return req.headers.authorization?.startsWith("Bearer fake-access-")
            ? answer(200, { user: FAKE_GOOGLE_USER })
            : answer(401, { error: { code: 401, message: "Invalid Credentials" } });
        case "GET /health":
          return answer(200, { ok: true });
        default:
          return answer(404, { error: "not_found" });
      }
    });
  });
  return { server, calls };
}

if (import.meta.main) {
  const port = Number(process.argv[2] ?? FAKE_GOOGLE_PORT);
  fakeGoogle().server.listen(port, "127.0.0.1", () => console.log(`Fake Google on http://127.0.0.1:${port}`));
}
