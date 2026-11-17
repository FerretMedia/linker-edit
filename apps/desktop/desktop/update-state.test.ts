import { describe, expect, it } from "vitest";
import {
  initialUpdateState,
  legacyUpdate,
  notAppliedVersion,
  onAvailable,
  onCheckFailed,
  onCheckStart,
  onDownloaded,
  onDownloadFailed,
  onInstallFailed,
  onNotAvailable,
  onProgress,
  releaseNoteLines,
} from "./update-state.ts";

const NOW = "2026-11-12T10:00:00.000Z";
const start = initialUpdateState("2026.1111.2");

describe("update state", () => {
  it("goes from checking to downloaded", () => {
    let state = onCheckStart(start);
    expect(state.status).toBe("checking");
    state = onAvailable(state, "2026.1112.0", ["Faster previews"], NOW);
    expect(state).toMatchObject({ status: "available", availableVersion: "2026.1112.0", notes: ["Faster previews"], checkedAt: NOW });
    state = onProgress(state, 41.7);
    expect(state).toMatchObject({ status: "downloading", percent: 41 });
    state = onDownloaded(state, "2026.1112.0");
    expect(state).toMatchObject({ status: "downloaded", downloadedVersion: "2026.1112.0", percent: 100 });
  });

  it("says up to date when there's nothing new", () => {
    expect(onNotAvailable(onCheckStart(start), NOW)).toMatchObject({ status: "up-to-date", availableVersion: null, checkedAt: NOW });
  });

  it("keeps a downloaded update through later checks, failed or not", () => {
    const downloaded = onDownloaded(onAvailable(start, "2026.1112.0", [], NOW), "2026.1112.0");
    expect(onCheckStart(downloaded).status).toBe("downloaded");
    expect(onNotAvailable(downloaded, NOW).status).toBe("downloaded");
    expect(onCheckFailed(downloaded, "offline", NOW)).toMatchObject({ status: "downloaded", error: null });
    expect(onAvailable(downloaded, "2026.1112.0", [], NOW).status).toBe("downloaded");
  });

  it("offers a newer release than the one downloaded", () => {
    const downloaded = onDownloaded(onAvailable(start, "2026.1112.0", [], NOW), "2026.1112.0");
    expect(onAvailable(downloaded, "2026.1112.1", [], NOW)).toMatchObject({ status: "available", downloadedVersion: null });
  });

  it("reports a failed check, and a failed download with the release still on offer", () => {
    expect(onCheckFailed(onCheckStart(start), "offline", NOW)).toMatchObject({ status: "error", error: "offline" });
    const available = onAvailable(start, "2026.1112.0", [], NOW);
    expect(onDownloadFailed(onProgress(available, 10), "reset")).toMatchObject({ status: "available", error: "reset", percent: null });
    expect(onDownloadFailed(start, "reset").status).toBe("error");
  });

  it("keeps the update after an installer that didn't start", () => {
    const downloaded = onDownloaded(start, "2026.1112.0");
    expect(onInstallFailed(downloaded, "spawn EACCES")).toMatchObject({ status: "downloaded", error: "spawn EACCES" });
  });

  it("carries a download link where the app can't install by itself", () => {
    const available = onAvailable(start, "2026.1112.0", [], NOW, "https://example.com/releases");
    expect(available.downloadUrl).toBe("https://example.com/releases");
    expect(legacyUpdate(available)).toEqual({ version: "2026.1112.0", state: "download", url: "https://example.com/releases" });
  });

  it("tells older pages about a ready update only", () => {
    expect(legacyUpdate(start)).toBeNull();
    expect(legacyUpdate(onAvailable(start, "2026.1112.0", [], NOW))).toBeNull();
    expect(legacyUpdate(onDownloaded(start, "2026.1112.0"))).toEqual({ version: "2026.1112.0", state: "ready" });
  });
});

describe("notAppliedVersion", () => {
  it("finds an install that came back on the old version", () => {
    expect(notAppliedVersion({ from: "2026.1111.2", to: "2026.1112.0" }, "2026.1111.2")).toBe("2026.1112.0");
  });

  it("is null once the install worked, or without a record", () => {
    expect(notAppliedVersion({ from: "2026.1111.2", to: "2026.1112.0" }, "2026.1112.0")).toBeNull();
    expect(notAppliedVersion(null, "2026.1111.2")).toBeNull();
    expect(notAppliedVersion({ from: 1 }, "2026.1111.2")).toBeNull();
  });
});

describe("releaseNoteLines", () => {
  // As electron-updater passes on a release with GitHub's generated notes.
  const generated = `<h2>What's Changed</h2>
<ul>
<li>Title bar built into the window, like T3 Code by <a class="user-mention notranslate" href="https://github.com/TheTrueFerret">@TheTrueFerret</a> in <a class="issue-link js-issue-link" href="https://github.com/FerretMedia/linker-edit/pull/7">#7</a></li>
<li>Name releases by their day: 2026.11.11, like Linker&#39;s by <a href="https://github.com/TheTrueFerret">@TheTrueFerret</a> in <a href="https://github.com/FerretMedia/linker-edit/pull/8">#8</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a class="commit-link" href="https://github.com/FerretMedia/linker-edit/compare/v1...v2"><tt>v1...v2</tt></a></p>`;

  it("lists the pull request titles", () => {
    expect(releaseNoteLines(generated)).toEqual([
      "Title bar built into the window, like T3 Code",
      "Name releases by their day: 2026.11.11, like Linker's",
    ]);
  });

  it("reads paragraphs when there's no list, without the changelog link", () => {
    expect(releaseNoteLines("<p>Our own Resolve MCP: 27 tools</p><p><strong>Full Changelog</strong>: v1...v2</p>")).toEqual([
      "Our own Resolve MCP: 27 tools",
    ]);
  });

  it("takes plain text and lists of notes", () => {
    expect(releaseNoteLines("First\nSecond")).toEqual(["First", "Second"]);
    expect(releaseNoteLines([{ version: "2", note: "<ul><li>New</li></ul>" }, { version: "1", note: null }])).toEqual(["New"]);
    expect(releaseNoteLines(null)).toEqual([]);
  });

  it("keeps it short", () => {
    const many = `<ul>${Array.from({ length: 10 }, (_, i) => `<li>Change ${i} ${"x".repeat(200)}</li>`).join("")}</ul>`;
    const lines = releaseNoteLines(many);
    expect(lines).toHaveLength(6);
    expect(lines[0]!.length).toBeLessThanOrEqual(140);
    expect(lines[0]!.endsWith("…")).toBe(true);
  });
});
