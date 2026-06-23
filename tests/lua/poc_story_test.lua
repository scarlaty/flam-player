-- Test : combien de spies sont safe a lire ?

function setup()
    local box = lv.obj.new(window)
    lv.obj.set_size(box, 290, 120)
    local btn = lv.btn.new(window)
    lv.obj.set_size(btn, 120, 35)

    -- Varier le nombre d'espions vs labels pour trouver le ratio safe
    -- Test : 5 espions, 10 labels
    local NUM = 5
    local spies = {}
    for i = 1, NUM do
        spies[i] = lv.label.new(window)
        lv.label.set_text(spies[i], ".")
        lv.obj.add_flag(spies[i], lv.OBJ_FLAG_HIDDEN)
    end
    for i = 1, NUM do
        lv.obj.del(spies[i])
    end

    -- 10 labels (plus que les 5 espions)
    local labels = {}
    local texts = {
        "Le Petit Prince", "Chapitre I", "Saint-Exupery",
        "Suite >", "Chapitre II", "Page 1",
        "Il etait une fois", "Un aviateur", "Le desert",
        "Une rose",
    }
    for i = 1, 10 do
        labels[i] = lv.label.new(window)
        lv.label.set_text(labels[i], texts[i])
        lv.obj.add_flag(labels[i], lv.OBJ_FLAG_HIDDEN)
    end

    -- Lire spy[1] seulement (on sait que ca marche)
    print("[DBG] Lecture spy[1] seulement...")
    local ok, txt = pcall(lv.label.get_text, spies[1])
    if ok then
        print(string.format("[DBG]  spy[1] → \"%s\"", txt))
    end

    -- Essayons les autres un par un via des appels separes
    -- pour voir le pattern
    print("[DBG] Lecture spy[3]...")
    ok, txt = pcall(lv.label.get_text, spies[3])
    if ok then print(string.format("[DBG]  spy[3] → \"%s\"", txt)) end

    print("[DBG] Lecture spy[5]...")
    ok, txt = pcall(lv.label.get_text, spies[5])
    if ok then print(string.format("[DBG]  spy[5] → \"%s\"", txt)) end

    print("[DBG] OK")
end
