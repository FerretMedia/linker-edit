// How processes started for the agents, such as their MCP servers, reach the Linker-Edit server.
import * as http from "node:http";

/** The env a child process needs to call postApi. */
export const apiEnv = (port: number, token: string) => ({ LINKER_EDIT_API: `http://127.0.0.1:${port}`, LINKER_EDIT_TOKEN: token });

// node:http rather than fetch: a first transcription downloads the engine, and an approval waits for the user,
// both longer than fetch's 5-minute header timeout.
export function postApi(route: string, body: unknown): Promise<{ status: number; text: string }> {
  return new Promise((resolve, reject) => {
    const request = http.request(
      `${process.env.LINKER_EDIT_API ?? ""}${route}`,
      { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${process.env.LINKER_EDIT_TOKEN ?? ""}` } },
      (response) => {
        let text = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => (text += chunk));
        response.on("end", () => resolve({ status: response.statusCode ?? 0, text }));
      },
    );
    request.on("error", reject);
    request.end(JSON.stringify(body));
  });
}
