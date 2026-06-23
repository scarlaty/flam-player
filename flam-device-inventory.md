# Inventaire FLAM Device (D:)

> Scan effectue le 2026-06-14

## Structure racine

```
D:/
├── .mdf                          (126 octets)
├── System Volume Information/
├── etc/
├── str/
├── tmp/
└── usr/
```

## `/etc/` — Configuration systeme

```
etc/
├── bluetooth/
│   ├── config/
│   │   ├── active                (5 octets)
│   │   └── last_device           (26 octets)
│   └── devices/
│       └── 0                     (26 octets)
├── library/
│   ├── list                      (370 octets)
│   └── list.hidden               (0 octets)
├── onboarding/
│   └── force_update              (5 octets)
├── update/
│   └── need_sync                 (5 octets)
└── wifi/
    └── networks/
        └── 0                     (52 octets)
```

## `/str/` — Histoires installees

10 histoires au format `.lsf` (Lua Script Format compile).

Chaque histoire contient : `info`, `key`, `main.lsf`, `version`, `img/`, `script/`, `sounds/`

| UUID | Version | Images | Scripts | Sons |
|------|---------|--------|---------|------|
| `4639e3ea-13b4-4550-91c9-f930a4df6b93` | 2 | 323 | 41 | 884 |
| `5fb70290-6246-419e-a764-bca1635fc850` | 2 | 64 | 33 | 152 |
| `76b88912-f3dc-41f4-b4a7-6aa809d57ad7` | 4 | 81 | 39 | 368 |
| `9a8521bf-0ae0-40de-8f8b-c22376bad8fd` | 1 | 123 | 30 | 510 |
| `a6c67c68-2db7-4713-9cf9-0d0e33336bc8` | 4 | 475 | 34 | 528 |
| `b0b3657c-9f0b-420a-969f-ffe7670e73d9` | 3 | 255 | 36 | 680 |
| `b2794eb7-82ba-4971-a567-e9f2880674b2` | 1 | 1 | 10 | 0 |
| `c103c95c-a852-4d8f-9c03-82f3814b572d` | 3 | 162 | 27 | 402 |
| `d2ccf774-b623-4e3c-908a-ae1bb78dcebf` | 3 | 163 | 33 | 678 |
| `f3f67d0d-79a6-4511-9cb2-389ac40001b9` | 1 | 55 | 35 | 102 |

> Note : `b2794eb7` est l'histoire "Les Magnons" transferee manuellement (pas de `sounds/`, 1 seule image).

## `/usr/` — Donnees utilisateur (sauvegardes)

```
usr/0/
├── library.cache                 (185 829 octets)
│
├── {uuid}.prog                   (4-6 octets, progression par histoire)
├── {uuid}.save                   (6 - 12 831 octets, etat sauvegarde)
└── {uuid}/
    └── chaps.save                (chapitres ecoutes)
```

### Sauvegardes par histoire

| UUID | .prog | .save | chaps.save |
|------|-------|-------|------------|
| `4639e3ea...` | 4 o | 12 195 o | 1 351 o |
| `5fb70290...` | 4 o | 3 327 o | 2 125 o |
| `76b88912...` | 4 o | 1 761 o | 1 970 o |
| `9a8521bf...` | 4 o | 12 831 o | 1 421 o |
| `a6c67c68...` | 4 o | 6 161 o | 2 254 o |
| `b0b3657c...` | 4 o | 6 208 o | 1 732 o |
| `b2794eb7...` | 4 o | 444 o | 6 o |
| `c103c95c...` | 4 o | 6 691 o | 1 381 o |
| `d2ccf774...` | 4 o | 1 154 o | 14 382 o |
| `f3f67d0d...` | 4 o | 2 092 o | 239 o |

UUIDs sans histoire installee (sauvegardes orphelines) :
- `00000000-0000-0000-0e11-000000000001`
- `1477a949-c556-450e-8d07-c2ff133b873d`
- `4d4d81ff-c465-41af-8eb3-8b3a2a258340`
- `5c0ffad6-985b-42f7-8ee2-a65313457aee`
- `60f3b2fb-1d0c-4329-b88a-d67d781ac791`
- `c23de5fb-52cb-41b6-aa2b-869e537512c9`
- `f0662db2-f32e-428c-81ee-2e65625aad3b`

## `/tmp/` — Fichiers temporaires

```
tmp/
└── bluetooth/
    └── scan/                     (vide)
```
