#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Cree un projet homebrew FLAM pret a builder (gabarit du skill flam-homebrew).

Usage :
    python new_project.py <dossier> --title "Mon jeu" [--subtitle "Jeu"]
                          [--module monjeu] [--version 1_0_0]

Cree :
    <dossier>/flam.json                     metadonnees (titre, uuid_seed unique, module)
    <dossier>/main.lua                      bootstrap (setup -> Global.load_module)
    <dossier>/script/<module>_<version>.lua module de jeu de demo (a remplacer)
    <dossier>/img/  <dossier>/sounds/       assets (PNG/LIF, MP3), vides
"""
import argparse
import json
import os
import re
import sys
import unicodedata
import uuid

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(errors="replace")   # console Windows cp1252 : pas de crash
    except AttributeError:
        pass

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(HERE, "..", "assets", "template")

# Noms deja pris par le runtime embarque (script/<nom>.lua) : un module de jeu
# du meme nom ecraserait un module du framework.
RUNTIME_DIR = os.path.join(HERE, "..", "assets", "runtime", "script")


def main():
    ap = argparse.ArgumentParser(description="Nouveau projet homebrew FLAM")
    ap.add_argument("dir", help="dossier du projet (cree s'il n'existe pas)")
    ap.add_argument("--title", required=True, help="titre affiche dans la bibliotheque")
    ap.add_argument("--subtitle", default="Jeu", help="sous-titre (defaut : Jeu)")
    ap.add_argument("--module", default=None,
                    help="nom du module Lua du jeu, [a-z0-9_] (defaut : derive du titre)")
    ap.add_argument("--version", default="1_0_0", help="version du module, ex. 1_0_0")
    args = ap.parse_args()

    module = args.module
    if module is None:
        ascii_title = unicodedata.normalize("NFKD", args.title).encode("ascii", "ignore").decode()
        module = re.sub(r"[^a-z0-9]+", "_", ascii_title.lower()).strip("_") or "game"
        if module[0].isdigit():
            module = "g_" + module
    if not re.fullmatch(r"[a-z][a-z0-9_]*", module):
        sys.exit("ERREUR : --module doit matcher [a-z][a-z0-9_]* (recu %r)" % module)
    if not re.fullmatch(r"[0-9]+(_[0-9]+)*", args.version):
        sys.exit("ERREUR : --version doit ressembler a 1_0_0 (recu %r)" % args.version)
    modfile = "%s_%s.lua" % (module, args.version)
    if os.path.isfile(os.path.join(RUNTIME_DIR, modfile)) or \
            os.path.isfile(os.path.join(RUNTIME_DIR, module + ".lua")):
        sys.exit("ERREUR : %s collisionne avec un module du runtime, choisir un autre --module"
                 % module)

    d = args.dir
    for f in ("flam.json", "main.lua", os.path.join("script", modfile)):
        if os.path.exists(os.path.join(d, f)):
            sys.exit("ERREUR : %s existe deja (aucun fichier ecrase)" % os.path.join(d, f))
    os.makedirs(os.path.join(d, "script"), exist_ok=True)
    os.makedirs(os.path.join(d, "img"), exist_ok=True)
    os.makedirs(os.path.join(d, "sounds"), exist_ok=True)

    meta = {
        "title": args.title,
        "subtitle": args.subtitle,
        # Graine de l'UUID (uuid.bin = md5(uuid_seed)). Unique par projet : la Flam
        # (et Lunii.QT) refusent un UUID deja installe. A garder stable entre deux
        # builds du meme jeu (meme sauvegarde), a changer pour un autre jeu.
        "uuid_seed": str(uuid.uuid4()),
        "module": module,
        "version": args.version,
    }
    with open(os.path.join(d, "flam.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=2)
        f.write("\n")

    def render(src, dst):
        with open(src, "r", encoding="utf-8") as f:
            txt = f.read()
        txt = txt.replace("__MODULE__", module).replace("__VERSION__", args.version)
        with open(dst, "w", encoding="utf-8", newline="\n") as f:
            f.write(txt)

    render(os.path.join(TEMPLATE, "main.lua"), os.path.join(d, "main.lua"))
    render(os.path.join(TEMPLATE, "script", "module_template.lua"),
           os.path.join(d, "script", modfile))

    print("OK -> %s" % os.path.abspath(d))
    print("  module : Global.load_module(%r, %r) -> script/%s" % (module, args.version, modfile))
    print("  build  : python %s %s" % (os.path.join(HERE, "flam_build.py"), d))


if __name__ == "__main__":
    main()
