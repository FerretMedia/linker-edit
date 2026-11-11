# set_color: a LUT or CDL on an AI timeline's clips, or the grade of one clip copied onto them.
# ARGS: timeline, item_ids, lut (path), cdl {slope, offset, power, saturation}, node, copy_from (item id)
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
items = items_by_id(t, ARGS.get("item_ids"))
lut, cdl, source = ARGS.get("lut"), ARGS.get("cdl"), ARGS.get("copy_from")
if not (lut or cdl or source):
    raise Refused("Pass lut, cdl or copy_from.")
node = int(ARGS.get("node") or 1)
refused = []


def triple(value, fallback):
    if value is None:
        return fallback
    if isinstance(value, (int, float)):
        value = [value] * 3
    return " ".join(str(float(v)) for v in value)


if source:
    origin = items_by_id(t, [source])[0]
    if not call(origin, "CopyGrades", items):
        refused.append("copy_from")
for item in items:
    item_id = call(item, "GetUniqueId")
    if lut:
        graph = call(item, "GetNodeGraph")
        if not graph or not call(graph, "SetLUT", node, lut):
            refused.append(f"{item_id}: lut")
    if cdl:
        values = {
            "NodeIndex": node,
            "Slope": triple(cdl.get("slope"), "1 1 1"),
            "Offset": triple(cdl.get("offset"), "0 0 0"),
            "Power": triple(cdl.get("power"), "1 1 1"),
            "Saturation": float(cdl.get("saturation", 1)),
        }
        if not call(item, "SetCDL", values):
            refused.append(f"{item_id}: cdl")
result = {"timeline": call(t, "GetName"), "clips": len(items)}
if refused:
    result["refused"] = refused
    result["note"] = "LUT paths are absolute or relative to Resolve's LUT folder (list_luts shows them); the node must exist."
