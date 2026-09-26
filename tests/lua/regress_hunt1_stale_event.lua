-- regress_hunt1_stale_event.lua — Non-regression hunt1 (bindings)
-- L'evenement passe aux callbacks est un light userdata vers le lv_event_t
-- local de lv_event_send. Garde apres le callback (saved = e) puis utilise
-- depuis un timer : lv.event.get_* lisaient une zone de pile morte (code
-- arbitraire, get_target => crash). Tout userdata etait aussi accepte.
-- Comportement attendu : erreur Lua "evenement expire" hors du callback.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_stale_event ---")

test("evenement conserve puis utilise depuis un timer", function()
    local o = lv.obj.new(window)
    local saved
    lv.obj.add_event_cb(o, function(e) saved = e end, lv.EVENT_CLICKED)
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_true(saved ~= nil, "callback non appele")
    local fired = false
    local r1, r2, r3
    lv.timer.new(function()
        fired = true
        r1 = pcall(lv.event.get_code, saved)
        r2 = pcall(lv.event.get_target, saved)
        r3 = pcall(lv.event.get_key_value, saved)
    end, 10, 1)
    test_tick(20)
    expect_true(fired, "timer non declenche")
    expect_eq(r1, false, "get_code accepte un evenement expire")
    expect_eq(r2, false, "get_target accepte un evenement expire")
    expect_eq(r3, false, "get_key_value accepte un evenement expire")
end)

test("autre userdata refuse comme evenement", function()
    local o = lv.obj.new(window)
    expect_error(function() lv.event.get_code(o) end, "LvObj accepte")
    expect_error(function() lv.event.get_target(lv.font.nunito_bold_12) end,
                 "police acceptee")
    expect_error(function() lv.event.get_code(nil) end, "nil accepte")
    expect_eq(lv.event.get_key_value(nil), "","get_key_value(nil)")
end)

test("evenement valide pendant le callback, y compris imbrique", function()
    local o = lv.obj.new(window)
    local p = lv.obj.new(window)
    local codes, targets = {}, {}
    lv.obj.add_event_cb(p, function(e)
        codes[#codes + 1] = lv.event.get_code(e)
        targets[#targets + 1] = lv.event.get_target(e)
    end, lv.EVENT_FOCUSED)
    lv.obj.add_event_cb(o, function(e)
        lv.event.send(p, lv.EVENT_FOCUSED)   -- imbrique
        codes[#codes + 1] = lv.event.get_code(e)
        targets[#targets + 1] = lv.event.get_target(e)
    end, lv.EVENT_CLICKED)
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_eq(codes[1], lv.EVENT_FOCUSED, "code imbrique")
    expect_eq(targets[1], p, "cible imbriquee")
    expect_eq(codes[2], lv.EVENT_CLICKED, "code apres imbrication")
    expect_eq(targets[2], o, "cible apres imbrication")
end)

test("erreur dans un callback : pile d'evenements depilee", function()
    local o = lv.obj.new(window)
    local saved
    lv.obj.add_event_cb(o, function(e) saved = e; error("boum") end, lv.EVENT_CLICKED)
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_error(function() lv.event.get_code(saved) end,
                 "evenement accepte apres un callback en erreur")
end)
