// A stand-in for Linker (app.linker.cool and its Clerk) as far as a signed-in Linker-Edit asks it: who the person
// is, and their orgs (GET /api/edit/orgs). The e2e server points linkerAuthServer and linkerAppUrl at it, with a
// sign-in written for it beforehand (signedInAccount). PUT /fake/orgs changes the orgs it lists.
//
//   node e2e/fake-linker.ts [port]
import * as http from "node:http";
import type { LinkerOrg } from "../shared/protocol.ts";

export const FAKE_LINKER_PORT = 4398;
const TOKEN = "fake-linker-token";

/** linker-account.json for a Linker-Edit signed in at the fake Linker on this port. */
export const signedInAccount = (port = FAKE_LINKER_PORT) => ({
  session: {
    authServer: `http://127.0.0.1:${port}`,
    client: { id: "fake-client" },
    accessToken: TOKEN,
    refreshToken: null,
    expiresAt: Date.UTC(2100, 0, 1),
    userId: "user_fake",
    name: "Jane Doe",
    email: "jane@example.com",
    picture: null,
    plan: "Max",
  },
});

export const FAKE_ORGS: LinkerOrg[] = [
  { id: "org_ferret", name: "Ferret Media", imageUrl: null, role: "admin" },
  { id: "org_north", name: "Studio North", imageUrl: null, role: "member" },
];

function serve(port: number) {
  let orgs = FAKE_ORGS;
  const base = `http://127.0.0.1:${port}`;
  const server = http.createServer((req, res) => {
    const answer = (status: number, body: unknown) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    const signedIn = req.headers.authorization === `Bearer ${TOKEN}`;
    const route = `${req.method} ${new URL(req.url ?? "/", base).pathname}`;
    switch (route) {
      case "GET /.well-known/oauth-authorization-server":
        return answer(200, { issuer: base, authorization_endpoint: `${base}/oauth/authorize`, token_endpoint: `${base}/oauth/token`, userinfo_endpoint: `${base}/oauth/userinfo` });
      case "GET /oauth/userinfo":
        return signedIn ? answer(200, { sub: "user_fake", name: "Jane Doe", email: "jane@example.com" }) : answer(401, { error: "invalid_token" });
      case "GET /api/connector":
        return signedIn ? answer(200, { plan: "Max" }) : answer(401, {});
      case "GET /api/edit/orgs":
        return signedIn ? answer(200, { orgs }) : answer(401, { error: "unauthorized" });
      case "PUT /fake/orgs": {
        let body = "";
        req.on("data", (chunk: Buffer) => (body += chunk));
        req.on("end", () => {
          orgs = JSON.parse(body) as LinkerOrg[];
          answer(200, { orgs });
        });
        return;
      }
      case "GET /health":
        return answer(200, { ok: true });
      default:
        return answer(404, { error: "not_found" });
    }
  });
  server.listen(port, "127.0.0.1", () => console.log(`Fake Linker on ${base}`));
}

if (import.meta.main) serve(Number(process.argv[2] ?? FAKE_LINKER_PORT));
