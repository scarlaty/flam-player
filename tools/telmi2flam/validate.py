#!/usr/bin/env python3
"""
validate.py — Validation hors-GUI d'une histoire FLAM (.plain.pk) generee.

Verifie :
  1. structure du .pk (entrees requises, methode stored)
  2. syntaxe Lua de main.lua et nodes.lua (compilation via lupa, sans execution)
  3. coherence : refs actions/stages, assets image/audio presents, accessibilite

Usage : python validate.py <histoire.plain.pk>
"""
import sys
import zipfile

from lupa import LuaRuntime, LuaError


def lua_to_py(v):
    """Convertit recursivement une table Lua (lupa) en dict/list Python."""
    if type(v).__name__ == "_LuaTable":
        keys = list(v.keys())
        # sequence 1..n -> liste
        if keys and all(isinstance(k, int) for k in keys) and \
           sorted(keys) == list(range(1, len(keys) + 1)):
            return [lua_to_py(v[k]) for k in range(1, len(keys) + 1)]
        return {k: lua_to_py(v[k]) for k in keys}
    return v


def main(pk_path):
    errs, warns, oks = [], [], []

    zf = zipfile.ZipFile(pk_path)
    infos = zf.infolist()
    names = set(i.filename for i in infos)

    # --- 1. structure ---
    nodes_name = "script/nodes.lua" if "script/nodes.lua" in names else "nodes.lua"
    required = ["info.plain", "version", "uuid.bin", "main.lua", nodes_name]
    for r in required:
        (oks if r in names else errs).append("entree %s" % r)
    nonstored = [i.filename for i in infos if i.compress_type != zipfile.ZIP_STORED]
    if nonstored:
        errs.append("%d entrees non-stored (ex: %s)" % (len(nonstored), nonstored[0]))
    else:
        oks.append("toutes les entrees en STORED")
    uuid = zf.read("uuid.bin")
    if len(uuid) == 16:
        oks.append("uuid.bin = 16 octets")
    else:
        errs.append("uuid.bin = %d octets (attendu 16)" % len(uuid))

    main_src = zf.read("main.lua").decode("utf-8", "replace")
    nodes_src = zf.read(nodes_name).decode("utf-8", "replace")

    # --- 2. syntaxe Lua (compile sans executer) ---
    lua = LuaRuntime(unpack_returned_tuples=True)
    try:
        lua.compile(main_src)
        oks.append("main.lua : syntaxe Lua valide")
    except LuaError as e:
        errs.append("main.lua : ERREUR syntaxe : %s" % e)
    try:
        lua.compile(nodes_src)
        oks.append("nodes.lua : syntaxe Lua valide")
    except LuaError as e:
        errs.append("nodes.lua : ERREUR syntaxe : %s" % e)

    # --- charger les donnees nodes.lua (data pure, sans effets de bord) ---
    data = None
    try:
        data = lua_to_py(lua.execute(nodes_src))
    except LuaError as e:
        errs.append("nodes.lua : impossible a charger : %s" % e)

    if data:
        stages = data.get("stages", {}) or {}
        actions = data.get("actions", {}) or {}
        oks.append("nodes.lua charge : %d stages, %d actions" % (len(stages), len(actions)))

        img_entries = set(n[len("img/"):] for n in names if n.startswith("img/"))
        snd_entries = set(n[len("sounds/"):] for n in names if n.startswith("sounds/"))

        def check_trans(label, tr):
            if not tr:
                return
            a = tr.get("action")
            if a not in actions:
                errs.append("%s -> action inconnue '%s'" % (label, a))

        missing_img, missing_aud, missing_map = set(), set(), set()
        bad_stage_ref = []

        for sid, st in stages.items():
            check_trans("stage %s.ok" % sid, st.get("ok"))
            check_trans("stage %s.home" % sid, st.get("home"))
            img = st.get("image")
            if img and img not in img_entries:
                missing_img.add(img)
            aud = st.get("audio")
            if aud:
                if aud not in snd_entries:
                    missing_aud.add(aud)
                if (aud + "map") not in snd_entries:
                    missing_map.add(aud)

        for aid, lst in actions.items():
            for e in lst:
                s = e.get("stage")
                if s not in stages:
                    bad_stage_ref.append("action %s -> stage inconnu '%s'" % (aid, s))

        # start
        start = data.get("start", {})
        if start.get("action") not in actions:
            errs.append("start -> action inconnue '%s'" % start.get("action"))
        else:
            oks.append("start -> action '%s' OK" % start.get("action"))

        # inventaire conditions : items hors bornes
        inv = data.get("inventory")
        ninv = len(inv) if inv else 0

        for m in bad_stage_ref:
            errs.append(m)
        if missing_img:
            errs.append("%d images manquantes dans le .pk (ex: %s)"
                        % (len(missing_img), sorted(missing_img)[:3]))
        else:
            oks.append("toutes les images referencees sont presentes")
        if missing_aud:
            errs.append("%d audios manquants (ex: %s)"
                        % (len(missing_aud), sorted(missing_aud)[:3]))
        else:
            oks.append("tous les audios referenced sont presents")
        if missing_map:
            warns.append("%d audios sans .mp3map (ex: %s)"
                         % (len(missing_map), sorted(missing_map)[:3]))
        else:
            oks.append("tous les audios ont un .mp3map")

        # --- accessibilite (BFS depuis start) ---
        def resolve_stages(tr):
            if not tr:
                return []
            return [e.get("stage") for e in actions.get(tr.get("action"), [])]

        seen = set()
        frontier = list(resolve_stages(data.get("start")))
        while frontier:
            s = frontier.pop()
            if s in seen or s not in stages:
                continue
            seen.add(s)
            st = stages[s]
            for tr in (st.get("ok"), st.get("home")):
                for ns in resolve_stages(tr):
                    if ns not in seen:
                        frontier.append(ns)
        unreached = set(stages) - seen
        if unreached:
            warns.append("%d stages non atteignables depuis start (ex: %s)"
                         % (len(unreached), sorted(unreached)[:5]))
        else:
            oks.append("tous les stages sont atteignables depuis start")

    zf.close()

    # --- rapport ---
    print("=== %s ===" % pk_path)
    for o in oks:
        print("  [OK]   %s" % o)
    for w in warns:
        print("  [WARN] %s" % w)
    for e in errs:
        print("  [ERR]  %s" % e)
    print("--- %d OK, %d warnings, %d erreurs ---" % (len(oks), len(warns), len(errs)))
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
