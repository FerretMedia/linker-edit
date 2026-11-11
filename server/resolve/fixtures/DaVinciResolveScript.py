# A stand-in for Resolve's scripting module, for testing Linker-Edit's scripts without Resolve. State comes
# from the JSON file in FAKE_RESOLVE; every call that changes something is logged to FAKE_RESOLVE + ".log".
import json
import os

STATE = json.load(open(os.environ["FAKE_RESOLVE"], encoding="utf8"))


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
        return 0


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


def scriptapp(name):
    return Resolve()
