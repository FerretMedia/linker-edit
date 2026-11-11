# A stand-in for Resolve's scripting module, for testing Linker-Edit's scripts without Resolve. State comes
# from the JSON file in FAKE_RESOLVE; every call that changes something is logged to FAKE_RESOLVE + ".log".
import json
import os

STATE = json.load(open(os.environ["FAKE_RESOLVE"], encoding="utf8"))
# A small grey PNG, for exported stills.
def png(width=4, height=4):
    import struct
    import zlib

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = b"".join(b"\x00" + b"\x80\x80\x80" * width for _ in range(height))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


PNG = png()


def log(*args):
    with open(os.environ["FAKE_RESOLVE"] + ".log", "a", encoding="utf8") as f:
        f.write(json.dumps(args) + "\n")


class Clip:
    def __init__(self, data):
        self.data = data

    def GetUniqueId(self):
        return self.data["id"]

    def GetName(self):
        return self.data["name"]

    def GetClipProperty(self, key=None):
        props = dict(self.data.get("props", {}), **{"File Path": self.data.get("path", "")})
        return props if key is None else props.get(key)

    def TranscribeAudio(self, speakers=None, nested=False):
        log("TranscribeAudio", self.data["id"], speakers)
        return True

    def GetTranscription(self, nested=False):
        return self.data.get("transcription")

    def SetClipColor(self, color):
        log("SetClipColor", self.data["id"], color)
        return color != "Rainbow"

    def AddFlag(self, color):
        log("AddFlag", self.data["id"], color)
        return True

    def SetMetadata(self, metadata):
        log("SetMetadata", self.data["id"], metadata)
        return True


class Folder:
    def __init__(self, data):
        self.data = data
        self.subs = [Folder(sub) for sub in data.get("folders", [])]
        self.clips = [Clip(clip) for clip in data.get("clips", [])]

    def GetName(self):
        return self.data["name"]

    def GetClipList(self):
        return self.clips

    def GetSubFolderList(self):
        return self.subs


class Item:
    def __init__(self, data, track_type, track, pool):
        self.data, self.track_type, self.track, self.pool = data, track_type, track, pool

    def GetUniqueId(self):
        return self.data["id"]

    def GetName(self):
        return self.data.get("name")

    def GetStart(self):
        return self.data["start"]

    def GetEnd(self):
        return self.data["end"]

    def GetSourceStartFrame(self):
        return self.data.get("source_start")

    def GetSourceEndFrame(self):
        return self.data.get("source_end")

    def GetMediaPoolItem(self):
        return self.pool.clip(self.data.get("media"))

    def GetClipEnabled(self):
        return self.data.get("enabled", True)

    def SetClipEnabled(self, value):
        log("SetClipEnabled", self.data["id"], value)
        return True

    def GetProperty(self, key=None):
        return self.data.get("properties", {"ZoomX": 1.0})

    def SetProperty(self, key, value):
        log("SetProperty", self.data["id"], key, value)
        return key != "Nonsense"

    def GetTrackTypeAndIndex(self):
        return [self.track_type, self.track]

    def GetFusionCompCount(self):
        return 1 if self.data.get("title") else 0

    def GetFusionCompByIndex(self, index):
        return Comp(self.data["id"]) if self.data.get("title") else None

    def GetNodeGraph(self, layer=None):
        return Graph(self.data["id"])

    def SetCDL(self, cdl):
        log("SetCDL", self.data["id"], cdl)
        return True

    def CopyGrades(self, items):
        log("CopyGrades", self.data["id"], [i.GetUniqueId() for i in items])
        return True


class Graph:
    def __init__(self, item_id):
        self.item_id = item_id

    def SetLUT(self, node, path):
        log("SetLUT", self.item_id, node, path)
        return path.endswith(".cube")


class TextTool:
    def __init__(self, item_id):
        self.item_id = item_id

    def SetInput(self, name, value):
        log("SetInput", self.item_id, name, value)
        return True


class Comp:
    def __init__(self, item_id):
        self.item_id = item_id

    def GetToolList(self, selected, kind):
        return {1: TextTool(self.item_id)} if kind == "TextPlus" else {}


class Timeline:
    def __init__(self, data, project):
        self.data, self.project = data, project

    def GetUniqueId(self):
        return self.data["id"]

    def GetName(self):
        return self.data["name"]

    def GetSetting(self, key):
        return {"timelineFrameRate": 25, "timelineResolutionWidth": "1920", "timelineResolutionHeight": "1080"}.get(key)

    def SetSetting(self, key, value):
        log("SetSetting", self.data["name"], key, value)
        return key != "timelineFrameRate"

    def GetStartFrame(self):
        return 90000

    def GetEndFrame(self):
        return 90000 + max([i["end"] - 90000 for t in self.data.get("video", []) for i in t] or [0])

    def GetStartTimecode(self):
        return "01:00:00:00"

    def tracks(self, kind):
        return self.data.setdefault(kind, [])

    def GetTrackCount(self, kind):
        return len(self.tracks(kind))

    def GetTrackName(self, kind, index):
        return f"{kind.title()} {index}"

    def GetItemListInTrack(self, kind, index):
        return [Item(i, kind, index, self.project.pool) for i in self.tracks(kind)[index - 1]]

    def AddTrack(self, kind, sub=None):
        log("AddTrack", self.data["name"], kind)
        self.tracks(kind).append([])
        return True

    def DeleteClips(self, items, ripple=False):
        log("DeleteClips", self.data["name"], [i.GetUniqueId() for i in items], ripple)
        return True

    def GetMarkers(self):
        return {str(m["frame"]): {k: v for k, v in m.items() if k != "frame"} for m in self.data.get("markers", [])}

    def AddMarker(self, frame, color, name, note, duration, custom=""):
        log("AddMarker", self.data["name"], frame, color, name, note, duration)
        return frame not in [m["frame"] for m in self.data.get("markers", [])]

    def GetCurrentTimecode(self):
        return self.data.get("playhead", "01:00:00:00")

    def SetCurrentTimecode(self, timecode):
        log("SetCurrentTimecode", self.data["name"], timecode)
        self.data["playhead"] = timecode
        return True

    def Export(self, path, kind, subtype):
        log("Export", self.data["name"], os.path.basename(path), kind, subtype)
        open(path, "w").write("exported")
        return True

    def SetName(self, name):
        log("SetName", self.data["name"], name)
        self.data["name"] = name
        return True

    def SetTrackName(self, kind, index, name):
        log("SetTrackName", kind, index, name)
        return True

    def SetTrackEnable(self, kind, index, value):
        log("SetTrackEnable", kind, index, value)
        return True

    def SetTrackLock(self, kind, index, value):
        log("SetTrackLock", kind, index, value)
        return kind != "subtitle"

    def DeleteMarkerAtFrame(self, frame):
        markers = self.data.get("markers", [])
        found = [m for m in markers if m["frame"] == frame]
        self.data["markers"] = [m for m in markers if m["frame"] != frame]
        return bool(found)

    def DeleteMarkersByColor(self, color):
        self.data["markers"] = [m for m in self.data.get("markers", []) if color != "All" and m["color"] != color]
        return True

    def InsertFusionTitleIntoTimeline(self, name):
        log("InsertFusionTitleIntoTimeline", self.data["name"], name, self.GetCurrentTimecode())
        if name != "Text+":
            return None
        return Item({"id": "title-1", "name": name, "start": 90100, "end": 90225, "title": True}, "video", 2, self.project.pool)

    def DetectSceneCuts(self):
        log("DetectSceneCuts", self.data["name"])
        self.data.setdefault("video", [[]])[0].append({"id": "cut", "start": 0, "end": 1})
        return True

    def DuplicateTimeline(self, name):
        log("DuplicateTimeline", self.data["name"], name)
        return self.project.add_timeline(dict(json.loads(json.dumps(self.data)), id=f"dup-{len(self.project.timelines)}", name=name))


class MediaPool:
    def __init__(self, project):
        self.project = project
        self.root = Folder(STATE["media"])
        self.current = self.root

    def clip(self, clip_id):
        def walk(folder):
            for c in folder.clips:
                if c.GetUniqueId() == clip_id:
                    return c
            for sub in folder.subs:
                found = walk(sub)
                if found:
                    return found
            return None

        return walk(self.root)

    def GetRootFolder(self):
        return self.root

    def GetCurrentFolder(self):
        return self.current

    def SetCurrentFolder(self, folder):
        log("SetCurrentFolder", folder.GetName())
        self.current = folder
        return True

    def AddSubFolder(self, parent, name):
        log("AddSubFolder", parent.GetName(), name)
        sub = Folder({"name": name})
        parent.subs.append(sub)
        return sub

    def MoveClips(self, clips, folder):
        log("MoveClips", [c.GetUniqueId() for c in clips], folder.GetName())
        return True

    def ImportTimelineFromFile(self, path, options=None):
        options = dict(options or {})
        options["sourceClipsFolders"] = [f.GetName() for f in options.get("sourceClipsFolders", [])]
        log("ImportTimelineFromFile", os.path.basename(path), options)
        if not path.endswith(".otio"):
            return None
        return self.project.add_timeline({"id": "imported", "name": "Timeline 1", "video": [[{"id": "x", "start": 90000, "end": 90010}]], "audio": []})

    def CreateEmptyTimeline(self, name, timecode=None):
        log("CreateEmptyTimeline", name)
        return self.project.add_timeline({"id": f"new-{len(self.project.timelines)}", "name": name, "video": [[]], "audio": [[]]})

    def AppendToTimeline(self, infos):
        timeline = self.project.current
        log("AppendToTimeline", timeline.GetName(), [dict(info, mediaPoolItem=info["mediaPoolItem"].GetUniqueId()) for info in infos])
        out = []
        for n, info in enumerate(infos):
            kind = {1: "video", 2: "audio"}.get(info.get("mediaType"), "video")
            track = info.get("trackIndex", 1)
            start = info.get("recordFrame", 90000)
            length = info.get("endFrame", 10) - info.get("startFrame", 0)
            out.append(Item({"id": f"added-{n}", "name": info["mediaPoolItem"].GetName(), "start": start, "end": start + length}, kind, track, self))
        return out


class MediaStorage:
    def AddItemListToMediaPool(self, paths):
        log("AddItemListToMediaPool", PROJECT.pool.current.GetName(), paths)
        return [Clip({"id": f"imported-{n}", "name": os.path.basename(p), "path": p}) for n, p in enumerate(paths)]


class Project:
    def __init__(self, data):
        self.data = data
        self.timelines = []
        self.pool = MediaPool(self)
        for timeline in data["timelines"]:
            self.add_timeline(timeline)
        self.current = next((t for t in self.timelines if t.GetName() == data.get("current")), None)

    def add_timeline(self, data):
        timeline = Timeline(data, self)
        self.timelines.append(timeline)
        return timeline

    def GetName(self):
        return self.data["name"]

    def GetUniqueId(self):
        return self.data["id"]

    def GetTimelineCount(self):
        return len(self.timelines)

    def GetTimelineByIndex(self, index):
        return self.timelines[index - 1]

    def GetCurrentTimeline(self):
        return self.current

    def SetCurrentTimeline(self, timeline):
        log("SetCurrentTimeline", timeline.GetName())
        self.current = timeline
        return True

    def GetMediaPool(self):
        return self.pool

    def GetSetting(self, key):
        return 25

    def GetTranscribeAudioStatus(self):
        return {"JobStatus": "Inactive"}

    def GetDetectSceneCutsStatus(self):
        return {"JobStatus": "Complete", "CompletionPercentage": 100}

    def ExportCurrentFrameAsStill(self, path):
        log("ExportCurrentFrameAsStill", self.current.GetCurrentTimecode(), os.path.basename(path))
        open(path, "wb").write(PNG)
        return True

    # Rendering: a job finishes the moment it starts, writing a file named as the settings say.
    def GetCurrentRenderFormatAndCodec(self):
        return {"format": "mov", "codec": "ProRes422HQ"}

    def GetRenderPresetList(self):
        return ["H.264 Master", "YouTube - 1080p"]

    def GetRenderFormats(self):
        return {"MP4": "mp4", "QuickTime": "mov", "Wave": "wav"}

    def GetRenderCodecs(self, fmt):
        return {
            "mp4": {"H.264": "H264", "H.265": "H265"},
            "mov": {"Apple ProRes 422 HQ": "ProRes422HQ"},
            "wav": {"Linear PCM": "LinearPCM"},
        }.get(fmt, {})

    def DeleteRenderPreset(self, name):
        return True

    def SaveAsNewRenderPreset(self, name):
        return True

    def LoadRenderPreset(self, name):
        log("LoadRenderPreset", name)
        return name in self.GetRenderPresetList() or name == "Linker-Edit restore"

    def SetCurrentRenderFormatAndCodec(self, fmt, codec):
        log("SetCurrentRenderFormatAndCodec", fmt, codec)
        STATE["render_ext"] = fmt
        return True

    def SetCurrentRenderMode(self, mode):
        return True

    def SetRenderSettings(self, settings):
        log("SetRenderSettings", settings)
        STATE["render_settings"] = settings
        return True

    def AddRenderJob(self):
        return "job-1"

    def GetRenderJobList(self):
        settings = STATE.get("render_settings", {})
        name = settings.get("CustomName")
        return [{"JobId": "job-1", "TargetDir": settings.get("TargetDir"), "OutputFilename": name + "." + STATE.get("render_ext", "mov")}]

    def StartRendering(self, jobs, interactive=False):
        job = self.GetRenderJobList()[0]
        open(os.path.join(job["TargetDir"], job["OutputFilename"]), "w").write("rendered")
        log("StartRendering", job["OutputFilename"])
        return True

    def IsRenderingInProgress(self):
        return False

    def GetRenderJobStatus(self, job):
        # Each script runs in a new process, so a finished job reads as done whatever came before.
        return {"JobStatus": "Complete", "CompletionPercentage": 100}

    def StopRendering(self):
        return True

    def DeleteRenderJob(self, job):
        log("DeleteRenderJob", job)
        return True


PROJECT = Project(STATE["project"])


class ProjectManager:
    def GetCurrentProject(self):
        return PROJECT

    def GetCurrentDatabase(self):
        return {"DbType": "Disk", "DbName": "Local Database"}


class Resolve:
    def GetProjectManager(self):
        return ProjectManager()

    def GetProductName(self):
        return "DaVinci Resolve Studio"

    def GetVersionString(self):
        return "21.1.2"

    def GetCurrentPage(self):
        return STATE.get("page", "edit")

    def GetMediaStorage(self):
        return MediaStorage()

    def OpenPage(self, page):
        log("OpenPage", page)
        return True

    EXPORT_OTIO = "otio"
    EXPORT_EDL = "edl"
    EXPORT_FCPXML_1_10 = "fcpxml"
    EXPORT_FCP_7_XML = "fcp7xml"
    EXPORT_AAF = "aaf"
    EXPORT_TEXT_CSV = "csv"
    EXPORT_DRT = "drt"
    EXPORT_NONE = "none"
    EXPORT_AAF_NEW = "aaf-new"


def scriptapp(name):
    return Resolve()
