#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Assemble le jeu casse-briques en contenu FLAM (dossier .plain + .plain.pk).

Reutilise le framework runtime de telmi2flam (script/*.lua, img/script/*.lif) et
y ajoute main.lua + script/game.lua + metadonnees. Aucun changement firmware.

Usage : python build.py [-o sortie.plain.pk] [--plain]
  defaut : ecrit _out/casse-briques.plain (dossier, pour le simulateur)
           ET _out/casse-briques.plain.pk (pour le device).
"""
import argparse
import hashlib
import os
import re
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
RUNTIME_SCRIPT = os.path.join(ROOT, "tools", "telmi2flam", "runtime", "script")
RUNTIME_IMG = os.path.join(ROOT, "tools", "telmi2flam", "runtime", "img", "script")

TITLE = "Casse-briques"
SUBTITLE = "Jeu"


def build():
    out_files = {}  # nom dans le .pk -> bytes

    # --- Code du jeu ---
    with open(os.path.join(HERE, "main.lua"), "rb") as f:
        out_files["main.lua"] = f.read()
    with open(os.path.join(HERE, "script", "breakout_1_0_0.lua"), "rb") as f:
        out_files["script/breakout_1_0_0.lua"] = f.read()

    # --- Framework runtime (global.lua, modules, ...) ---
    nlib = 0
    for fn in sorted(os.listdir(RUNTIME_SCRIPT)):
        if fn.endswith(".lua"):
            with open(os.path.join(RUNTIME_SCRIPT, fn), "rb") as f:
                out_files["script/" + fn] = f.read()
            nlib += 1

    # --- Assets UI (fleches, empty.lif, ...) requis par certains modules ---
    nui = 0
    for fn in sorted(os.listdir(RUNTIME_IMG)):
        if fn.endswith(".lif"):
            with open(os.path.join(RUNTIME_IMG, fn), "rb") as f:
                out_files["img/script/" + fn] = f.read()
            nui += 1
    # img/empty.lif a la racine img/ (convention des histoires)
    empty = os.path.join(RUNTIME_IMG, "empty.lif")
    if os.path.isfile(empty):
        with open(empty, "rb") as f:
            out_files["img/empty.lif"] = f.read()

    # --- MP3 silencieux (au cas ou un module l'exige) ---
    silent_frame = bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(413)
    out_files["sounds/silent.mp3"] = silent_frame * 24

    # --- Metadonnees FLAM ---
    uuid16 = hashlib.md5(TITLE.encode("utf-8")).digest()
    info = "%s\n%s\n000000\n%s" % (TITLE, SUBTITLE, TITLE)
    out_files["info.plain"] = info.encode("utf-8")
    out_files["version"] = b"1"
    out_files["uuid.bin"] = uuid16

    return out_files, nlib, nui


def write(out_files, out_path, emit_plain):
    dir_only = out_path.endswith(".plain")

    head = ["info.plain", "version", "main.lua", "script/breakout_1_0_0.lua"]
    order = [n for n in head if n in out_files]
    order += sorted(n for n in out_files if n not in head and n != "uuid.bin")
    if "uuid.bin" in out_files:
        order.append("uuid.bin")

    written = []
    if not dir_only:
        os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
        with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as out:
            for name in order:
                out.writestr(name, out_files[name])
        written.append(out_path)

    if emit_plain or dir_only:
        if dir_only:
            plain_dir = out_path
        elif out_path.endswith(".plain.pk"):
            plain_dir = out_path[:-3]
        else:
            plain_dir = out_path + ".plain"
        for name in order:
            dest = os.path.join(plain_dir, *name.split("/"))
            os.makedirs(os.path.dirname(dest) or ".", exist_ok=True)
            with open(dest, "wb") as fh:
                fh.write(out_files[name])
        written.append(plain_dir + os.sep)
    return written


def main():
    ap = argparse.ArgumentParser(description="Build casse-briques (.plain / .plain.pk)")
    ap.add_argument("-o", "--output", help="sortie .plain.pk ou dossier .plain")
    ap.add_argument("--plain", action="store_true", help="ecrire AUSSI le dossier .plain")
    args = ap.parse_args()

    out_files, nlib, nui = build()

    out_path = args.output
    emit_plain = args.plain
    if out_path is None:
        # Defaut : dossier .plain (sim) + .plain.pk (device) dans _out/
        out_dir = os.path.join(HERE, "_out")
        write(out_files, os.path.join(out_dir, "casse-briques.plain"), False)
        written = write(out_files, os.path.join(out_dir, "casse-briques.plain.pk"), False)
        written.insert(0, os.path.join(out_dir, "casse-briques.plain") + os.sep)
    else:
        written = write(out_files, out_path, emit_plain)

    for w in written:
        print("OK -> %s" % w)
    print("  titre : %s | scripts framework : %d | assets UI : %d" % (TITLE, nlib, nui))


if __name__ == "__main__":
    main()
