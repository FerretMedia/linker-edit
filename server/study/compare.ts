// Raw footage against the finished video, by what is said: lines the final cut's words up with the raw footage's,
// which shows what the editor kept, what they cut, which take they used and in what order. No project file needed.
import type { Transcript } from "../transcribe/whisper.ts";

interface Token {
  word: string;
  start: number;
  end: number;
}

export interface KeptPart {
  /** Seconds in the finished video. */
  final: [number, number];
  /** The raw file it comes from, and where in it, in seconds. */
  raw_file: string;
  raw: [number, number];
  words: number;
  text: string;
  /** How often the raw footage has these words, when more than once: which take this is. */
  take?: { used: number; of: number };
}

export interface CutPart {
  raw_file: string;
  raw: [number, number];
  words: number;
  text: string;
  /** "retake": most of it is said again in a part that was kept. "content": it isn't in the finished video at all. */
  kind: "retake" | "content";
}

export interface CutComparison {
  final_seconds: number;
  raw_seconds: number;
  raw_words: number;
  /** How much of what was said in the raw footage is in the finished video, 0 to 1. */
  kept_share: number;
  /** How much of what is said in the finished video wasn't found in the raw footage (voice-over, other footage), 0 to 1. */
  not_from_raw_share: number;
  parts: number;
  average_part_seconds: number;
  /** The silence left where two parts meet, in the finished video. */
  median_gap_at_cuts_seconds: number;
  /** How often the finished video jumps back in the raw footage, or to another file: the editor reordered. */
  reordered: number;
  kept: KeptPart[];
  cut: CutPart[];
  /** More parts than listed. */
  truncated?: boolean;
}

const GRAM = 3;
const MIN_RUN = 4;
const MAX_LISTED = 60;
// A cut stretch shorter than this is a word or two trimmed, not a decision worth listing.
const MIN_CUT_WORDS = 6;

const round = (value: number, digits = 2) => Math.round(value * 10 ** digits) / 10 ** digits;

/** Words as they are compared: lower case, no punctuation. Words that are only punctuation are dropped. */
export function tokens(transcript: Transcript): Token[] {
  return transcript.segments
    .flatMap((segment) => segment.words)
    .map((word) => ({ word: word.text.toLowerCase().replace(/[^\p{L}\p{N}']+/gu, ""), start: word.start, end: word.end }))
    .filter((token) => token.word);
}

const gramAt = (list: Token[], index: number) => `${list[index]!.word} ${list[index + 1]!.word} ${list[index + 2]!.word}`;

function snippet(list: Token[], from: number, to: number): string {
  const words = list.slice(from, to).map((token) => token.word);
  return words.length <= 16 ? words.join(" ") : `${words.slice(0, 8).join(" ")} … ${words.slice(-6).join(" ")}`;
}

interface Run {
  finalFrom: number;
  finalTo: number;
  file: number;
  rawFrom: number;
  rawTo: number;
  takes: number[];
}

/** Compares the finished video's transcript with the raw footage's. */
export function compareCut(final: Transcript, raw: Transcript[]): CutComparison {
  const finalTokens = tokens(final);
  const rawTokens = raw.map(tokens);

  // Where each run of three words is said in the raw footage.
  const index = new Map<string, { file: number; at: number }[]>();
  rawTokens.forEach((list, file) => {
    for (let at = 0; at + GRAM <= list.length; at += 1) {
      const key = gramAt(list, at);
      const places = index.get(key);
      if (places) places.push({ file, at });
      else index.set(key, [{ file, at }]);
    }
  });

  const matchLength = (finalAt: number, file: number, rawAt: number) => {
    const list = rawTokens[file]!;
    const same = (offset: number) =>
      finalAt + offset < finalTokens.length && rawAt + offset < list.length && finalTokens[finalAt + offset]!.word === list[rawAt + offset]!.word;
    let length = 0;
    // One word heard differently in the two transcripts doesn't end a match when the next two agree again.
    while (same(length) || (length > 0 && same(length + 1) && same(length + 2))) length += 1;
    return length;
  };

  const runs: Run[] = [];
  let at = 0;
  while (at + GRAM <= finalTokens.length) {
    const places = index.get(gramAt(finalTokens, at)) ?? [];
    const previous = runs.at(-1);
    let best: { file: number; at: number; length: number } | null = null;
    const lengths = places.map((place) => ({ ...place, length: matchLength(at, place.file, place.at) }));
    for (const candidate of lengths) {
      if (candidate.length < MIN_RUN) continue;
      // The longest match wins; between equals, the one that goes on from where the last part ended.
      const follows = (place: { file: number; at: number }) =>
        previous && place.file === previous.file && place.at >= previous.rawTo ? place.at - previous.rawTo : Number.MAX_SAFE_INTEGER;
      if (!best || candidate.length > best.length || (candidate.length === best.length && follows(candidate) < follows(best))) best = candidate;
    }
    if (!best) {
      at += 1;
      continue;
    }
    // The same words said elsewhere in the raw footage are other takes of this part.
    const length = best.length;
    const takes = lengths
      .filter((candidate) => candidate.length >= Math.min(length, 8))
      .sort((a, b) => a.file - b.file || a.at - b.at)
      .map((candidate) => candidate.file * 1e9 + candidate.at);
    runs.push({ finalFrom: at, finalTo: at + length, file: best.file, rawFrom: best.at, rawTo: best.at + length, takes });
    at += length;
  }

  // A misheard word splits one kept part in two; parts that carry on in the raw footage are one.
  const merged: Run[] = [];
  for (const run of runs) {
    const last = merged.at(-1);
    const list = rawTokens[run.file]!;
    const continues =
      last &&
      last.file === run.file &&
      run.rawFrom >= last.rawTo &&
      run.rawFrom - last.rawTo <= 3 &&
      run.finalFrom - last.finalTo <= 3 &&
      list[run.rawFrom]!.start - list[last.rawTo - 1]!.end < 1;
    if (continues) {
      last.finalTo = run.finalTo;
      last.rawTo = run.rawTo;
    } else merged.push({ ...run });
  }

  const kept: KeptPart[] = merged.map((run) => {
    const list = rawTokens[run.file]!;
    const used = run.takes.indexOf(run.file * 1e9 + run.rawFrom) + 1;
    return {
      final: [round(finalTokens[run.finalFrom]!.start), round(finalTokens[run.finalTo - 1]!.end)],
      raw_file: raw[run.file]!.file,
      raw: [round(list[run.rawFrom]!.start), round(list[run.rawTo - 1]!.end)],
      words: run.finalTo - run.finalFrom,
      text: snippet(finalTokens, run.finalFrom, run.finalTo),
      ...(run.takes.length > 1 && used > 0 ? { take: { used, of: run.takes.length } } : {}),
    };
  });

  // What's left of the raw footage, and whether it was said again in what was kept.
  const finalGrams = new Set<string>();
  for (let i = 0; i + GRAM <= finalTokens.length; i += 1) finalGrams.add(gramAt(finalTokens, i));
  const cut: CutPart[] = [];
  rawTokens.forEach((list, file) => {
    const covered = new Array<boolean>(list.length).fill(false);
    for (const run of merged) if (run.file === file) covered.fill(true, run.rawFrom, run.rawTo);
    let from = 0;
    while (from < list.length) {
      if (covered[from]) {
        from += 1;
        continue;
      }
      let to = from;
      while (to < list.length && !covered[to]) to += 1;
      if (to - from >= MIN_CUT_WORDS) {
        let repeated = 0;
        const grams = to - from - GRAM + 1;
        for (let i = from; i + GRAM <= to; i += 1) if (finalGrams.has(gramAt(list, i))) repeated += 1;
        cut.push({
          raw_file: raw[file]!.file,
          raw: [round(list[from]!.start), round(list[to - 1]!.end)],
          words: to - from,
          text: snippet(list, from, to),
          kind: grams > 0 && repeated / grams >= 0.5 ? "retake" : "content",
        });
      }
      from = to;
    }
  });

  const keptWords = merged.reduce((sum, run) => sum + (run.rawTo - run.rawFrom), 0);
  const matchedFinal = merged.reduce((sum, run) => sum + (run.finalTo - run.finalFrom), 0);
  const rawWords = rawTokens.reduce((sum, list) => sum + list.length, 0);
  const gaps = kept
    .slice(1)
    .map((part, i) => part.final[0] - kept[i]!.final[1])
    .filter((gap) => gap >= 0)
    .sort((a, b) => a - b);
  const reordered = merged.slice(1).filter((run, i) => run.file !== merged[i]!.file || run.rawFrom < merged[i]!.rawTo).length;
  const partSeconds = kept.reduce((sum, part) => sum + (part.final[1] - part.final[0]), 0);

  return {
    final_seconds: round(final.duration),
    raw_seconds: round(raw.reduce((sum, transcript) => sum + transcript.duration, 0)),
    raw_words: rawWords,
    kept_share: rawWords ? round(keptWords / rawWords) : 0,
    not_from_raw_share: finalTokens.length ? round(1 - matchedFinal / finalTokens.length) : 0,
    parts: kept.length,
    average_part_seconds: kept.length ? round(partSeconds / kept.length) : 0,
    median_gap_at_cuts_seconds: round(gaps[Math.floor(gaps.length / 2)] ?? 0),
    reordered,
    kept: kept.slice(0, MAX_LISTED),
    cut: cut.slice(0, MAX_LISTED),
    ...(kept.length > MAX_LISTED || cut.length > MAX_LISTED ? { truncated: true } : {}),
  };
}
