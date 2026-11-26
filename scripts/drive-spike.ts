// Throwaway spike. Under the drive.file scope, does picking a folder in Google's desktop Picker
// let the app read what is in it, including files added later?
//
//   node scripts/drive-spike.ts            sign in, pick a folder, test it
//   node scripts/drive-spike.ts --relist   test the same folder again, without the Picker
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { parseArgs } from "node:util";

const SCOPE = "https://www.googleapis.com/auth/drive.file";
const FILES = "https://www.googleapis.com/drive/v3/files";
const FOLDER = "application/vnd.google-apps.folder";
/** The app can always see files it made, so these never count as proof of access. */
const OWN_PREFIX = "linker-edit-spike-";
const STATE_FILE = path.join(os.tmpdir(), "linker-edit-drive-spike.json");

const SETUP = `Set LINKER_EDIT_GOOGLE_CLIENT_ID and LINKER_EDIT_GOOGLE_CLIENT_SECRET first.

Make a Google OAuth client (once):
  1. Open https://console.cloud.google.com and create or select a project.
  2. APIs & Services > Library: enable "Google Drive API" and "Google Picker API".
  3. Google Auth Platform: fill in Branding if asked. Under Audience choose External,
     leave the status on Testing, and add your own Google account under Test users.
  4. Google Auth Platform > Clients > Create client > Application type "Desktop app".
     Copy the client ID and the client secret.
  5. Set them in the terminal you run this from.
     PowerShell:
       $env:LINKER_EDIT_GOOGLE_CLIENT_ID = "<client id>"
       $env:LINKER_EDIT_GOOGLE_CLIENT_SECRET = "<client secret>"
     bash:
       export LINKER_EDIT_GOOGLE_CLIENT_ID="<client id>"
       export LINKER_EDIT_GOOGLE_CLIENT_SECRET="<client secret>"

Make a test folder in drive.google.com (not with this script):
  - one uploaded file in it, such as a .txt or an image (not a Google Doc)
  - one subfolder, with another uploaded file in that

Then run:
  node scripts/drive-spike.ts            pick only the test folder, none of its files
  (add one more file to the folder in drive.google.com)
  node scripts/drive-spike.ts --relist`;

interface Saved {
	refreshToken: string;
	pickedIds: string[];
	/** Everything listed right after the Picker, so --relist can tell what is new. */
	seenAtSignIn: string[];
}

interface DriveFile {
	id: string;
	name: string;
	mimeType: string;
	size?: string;
}

interface Tokens {
	access_token: string;
	refresh_token?: string;
	scope: string;
}

async function requestTokens(params: Record<string, string>): Promise<Tokens> {
	const response = await fetch("https://oauth2.googleapis.com/token", {
		method: "POST",
		body: new URLSearchParams(params),
	});
	if (!response.ok) throw new Error(`Google's token endpoint said ${response.status}: ${await response.text()}`);
	return (await response.json()) as Tokens;
}

function openBrowser(url: string): void {
	const [command, ...args] =
		process.platform === "win32"
			? ["rundll32", "url.dll,FileProtocolHandler"]
			: [process.platform === "darwin" ? "open" : "xdg-open"];
	spawn(command!, [...args, url], { stdio: "ignore", detached: true })
		.on("error", () => console.log("Couldn't open a browser. Open the link above yourself."))
		.unref();
}

async function pick(clientId: string, clientSecret: string): Promise<{ tokens: Tokens; pickedIds: string[] }> {
	const verifier = randomBytes(32).toString("base64url");
	const state = randomBytes(16).toString("base64url");
	const server = http.createServer();
	await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
	const redirectUri = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;

	const redirected = new Promise<URLSearchParams>((resolve) => {
		server.on("request", (request, response) => {
			const query = new URL(request.url ?? "/", redirectUri).searchParams;
			// The Picker docs' example redirect has no state, so only a wrong one is refused. PKCE covers the code.
			if ((query.get("state") ?? state) !== state || !(query.has("code") || query.has("error"))) {
				response.writeHead(404).end();
				return;
			}
			response.writeHead(200, { "content-type": "text/plain", connection: "close" });
			response.end("Done. Go back to the terminal.", () => {
				server.close();
				server.closeAllConnections();
				resolve(query);
			});
		});
	});

	const url = `https://accounts.google.com/o/oauth2/v2/auth?${new URLSearchParams({
		client_id: clientId,
		redirect_uri: redirectUri,
		response_type: "code",
		scope: SCOPE,
		access_type: "offline",
		prompt: "consent",
		trigger_onepick: "true",
		allow_folder_selection: "true",
		allow_multiple: "true",
		state,
		code_challenge: createHash("sha256").update(verifier).digest("base64url"),
		code_challenge_method: "S256",
	})}`;
	console.log(`Pick your test folder in the browser. If nothing opens, use this link:\n${url}\n`);
	openBrowser(url);

	const query = await redirected;
	const code = query.get("code");
	if (!code) throw new Error(`Google redirected with an error: ${query.get("error")}`);
	const tokens = await requestTokens({
		client_id: clientId,
		client_secret: clientSecret,
		code,
		code_verifier: verifier,
		grant_type: "authorization_code",
		redirect_uri: redirectUri,
	});
	const pickedIds = (query.get("picked_file_ids") ?? "").split(",").filter(Boolean);
	if (pickedIds.length === 0) {
		throw new Error(`The redirect had no picked_file_ids. It had: ${[...query.keys()].join(", ")}`);
	}
	return { tokens, pickedIds };
}

type Answer = { ok: true; response: Response } | { ok: false; status: string };

async function drive(accessToken: string, url: string, init: RequestInit = {}): Promise<Answer> {
	const response = await fetch(url, {
		...init,
		headers: { ...(init.headers as Record<string, string>), authorization: `Bearer ${accessToken}` },
	});
	if (response.ok) return { ok: true, response };
	const body = await response.text();
	let reason: string | undefined;
	try {
		reason = (JSON.parse(body) as { error?: { errors?: { reason?: string }[] } }).error?.errors?.[0]?.reason;
	} catch {
		// Not every error body is JSON; the status alone still answers the question.
	}
	return { ok: false, status: reason ? `${response.status} ${reason}` : `${response.status}` };
}

function getFile(accessToken: string, id: string): Promise<Answer> {
	return drive(accessToken, `${FILES}/${id}?fields=id,name,mimeType,size&supportsAllDrives=true`);
}

async function list(accessToken: string, parentId: string): Promise<{ files: DriveFile[] } | { status: string }> {
	const files: DriveFile[] = [];
	let pageToken = "";
	do {
		const params = new URLSearchParams({
			q: `'${parentId}' in parents and trashed=false`,
			fields: "nextPageToken,files(id,name,mimeType,size)",
			pageSize: "1000",
			supportsAllDrives: "true",
			includeItemsFromAllDrives: "true",
			...(pageToken && { pageToken }),
		});
		const answer = await drive(accessToken, `${FILES}?${params}`);
		if (!answer.ok) return { status: answer.status };
		const page = (await answer.response.json()) as { files: DriveFile[]; nextPageToken?: string };
		files.push(...page.files.filter((file) => !file.name.startsWith(OWN_PREFIX)));
		pageToken = page.nextPageToken ?? "";
	} while (pageToken);
	return { files };
}

async function download(accessToken: string, file: DriveFile): Promise<string> {
	const answer = await drive(accessToken, `${FILES}/${file.id}?alt=media&supportsAllDrives=true`, {
		headers: { range: "bytes=0-0" },
	});
	if (!answer.ok) return `${answer.status} (${file.name})`;
	await answer.response.body?.cancel();
	return `ok (${file.name})`;
}

async function create(accessToken: string, parentId: string): Promise<string> {
	const name = `${OWN_PREFIX}${Date.now()}.txt`;
	const answer = await drive(accessToken, `${FILES}?supportsAllDrives=true`, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify({ name, parents: [parentId] }),
	});
	if (!answer.ok) return answer.status;
	const { id } = (await answer.response.json()) as { id: string };
	const removed = await drive(accessToken, `${FILES}/${id}?supportsAllDrives=true`, { method: "DELETE" });
	return removed.ok ? "ok (then deleted it)" : `ok (delete ${name} yourself: ${removed.status})`;
}

/** Tests one picked id, prints its table, and returns the ids it could list. */
async function inspect(accessToken: string, id: string, seenBefore: Set<string> | null): Promise<string[]> {
	const names: string[] = [];
	const seen: string[] = [];
	const note = (files: DriveFile[], prefix: string) => {
		for (const file of files) {
			seen.push(file.id);
			const isNew = seenBefore && !seenBefore.has(file.id) ? "   <- new since sign-in" : "";
			names.push(`${prefix}${file.name}${file.mimeType === FOLDER ? "/" : ""}${isNew}`);
		}
	};

	const meta = await getFile(accessToken, id);
	const picked = meta.ok ? ((await meta.response.json()) as DriveFile) : null;

	const top = await list(accessToken, id);
	const children = "files" in top ? top.files : [];
	note(children, "");
	const topFiles = children.filter((file) => file.mimeType !== FOLDER);
	const subfolders = children.filter((file) => file.mimeType === FOLDER);

	const nested: DriveFile[] = [];
	const failures: string[] = [];
	for (const folder of subfolders) {
		const inside = await list(accessToken, folder.id);
		if ("status" in inside) {
			failures.push(`${inside.status} (${folder.name})`);
			continue;
		}
		note(inside.files, `${folder.name}/`);
		nested.push(...inside.files.filter((file) => file.mimeType !== FOLDER));
	}

	let nestedRow = "not tested: no file listed in a subfolder";
	if (nested[0]) {
		const answer = await getFile(accessToken, nested[0].id);
		nestedRow = `${answer.ok ? "ok" : answer.status} (${nested[0].name})`;
	}

	// Google Docs and shortcuts have no bytes to download, and an empty file has no first byte.
	const downloadable = [...topFiles, ...nested].find(
		(file) => !file.mimeType.startsWith("application/vnd.google-apps.") && file.size !== "0",
	);

	const rows: [string, string][] = [
		["picked folder metadata", meta.ok ? "ok" : meta.status],
		[
			"files inside",
			"status" in top
				? top.status
				: topFiles.length
					? `ok (${topFiles.length} listed)`
					: "200, but no files listed",
		],
		[
			"a subfolder",
			failures.length
				? failures.join(", ")
				: subfolders.length
					? `ok (${subfolders.length} opened)`
					: "not tested: no subfolder listed",
		],
		["a file inside the subfolder", nestedRow],
		["download", downloadable ? await download(accessToken, downloadable) : "not tested: no downloadable file listed"],
		["creating a file", await create(accessToken, id)],
	];

	console.log(`\nPicked ${picked ? `"${picked.name}"` : "item"} (${id})`);
	if (picked && picked.mimeType !== FOLDER) console.log("This is a file, not a folder. Pick a folder to answer the question.");
	for (const [label, result] of rows) console.log(`  ${label.padEnd(28)} ${result}`);
	console.log(names.length ? `  Listed:\n    ${names.join("\n    ")}` : "  Listed: nothing");
	if (seenBefore && !seen.some((fileId) => !seenBefore.has(fileId))) {
		console.log("  Nothing new is listed since sign-in.");
	}
	return seen;
}

async function main(): Promise<void> {
	const { values } = parseArgs({ options: { relist: { type: "boolean", default: false } } });
	const clientId = process.env.LINKER_EDIT_GOOGLE_CLIENT_ID;
	const clientSecret = process.env.LINKER_EDIT_GOOGLE_CLIENT_SECRET;
	if (!clientId || !clientSecret) {
		console.log(SETUP);
		process.exitCode = 1;
		return;
	}

	let accessToken: string;
	let saved: Saved;
	if (values.relist) {
		if (!fs.existsSync(STATE_FILE)) throw new Error(`No ${STATE_FILE}. Run once without --relist first.`);
		saved = JSON.parse(fs.readFileSync(STATE_FILE, "utf8")) as Saved;
		({ access_token: accessToken } = await requestTokens({
			client_id: clientId,
			client_secret: clientSecret,
			refresh_token: saved.refreshToken,
			grant_type: "refresh_token",
		}));
	} else {
		const { tokens, pickedIds } = await pick(clientId, clientSecret);
		if (!tokens.refresh_token) throw new Error("Google sent no refresh token.");
		accessToken = tokens.access_token;
		saved = { refreshToken: tokens.refresh_token, pickedIds, seenAtSignIn: [] };
		fs.writeFileSync(STATE_FILE, JSON.stringify(saved), { mode: 0o600 });
		console.log(`Granted scope: ${tokens.scope}`);
		console.log(`Refresh token saved to ${STATE_FILE} (delete it when you're done).`);
	}

	const seenBefore = values.relist ? new Set(saved.seenAtSignIn) : null;
	const seen: string[] = [];
	for (const id of saved.pickedIds) seen.push(...(await inspect(accessToken, id, seenBefore)));

	if (!values.relist) {
		fs.writeFileSync(STATE_FILE, JSON.stringify({ ...saved, seenAtSignIn: seen }), { mode: 0o600 });
		console.log(
			"\nNow add a file to the picked folder in drive.google.com, then run:\n  node scripts/drive-spike.ts --relist",
		);
	}
}

await main();
