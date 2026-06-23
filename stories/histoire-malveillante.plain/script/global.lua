local enableDebug = false
---@class global
local global = {}

global.screen_width = 320
global.screen_height = 240

global.header_height = 28

global.visual_width = global.screen_width
global.visual_height = global.screen_height - global.header_height

global.current_module = nil
global.current_module_name = nil

global.loadedResources = {}

global.current_branch = nil
global.current_branch_name = nil
global.isMultiBranches = false

global.progression = require("progressionManager")

global.button = require("button")
global.button_theme_default = require("button-theme-default")
global.button_theme_interactive = require("button-theme-interactive")
global.v_scroll = require("v-scroll")
global.v_container = require("v-container")

global.audioDelayTimer = nil
global.audioDelayValue = 500
global.audioDelayPath = nil
global.audioFeedbackCallback = nil
global.audioDuration = nil
global.audioCB = nil
global.audioNextHavePriority = false

global.canShowPausePanel = true
global.pauseContainerStyle = nil
global.pauseContainer = nil

-- Default audioPlayer images
global.apDefaultCover = "player_cover.lif"
global.apDefaultBGCover = "Player_BG.lif"
global.apDefaultFGCover = "Player_FG.lif"

function global.setBackBehavior(backBehavior, backBehaviorArgs)
    back_callback = function()
        global.requestAudioStop(true, true)
        global.cleanCurrentModule()

        if (backBehaviorArgs ~= nil) then
            backBehavior(backBehaviorArgs)
        else
            backBehavior()
        end
    end
end

function global.setBackModule(backModule)
    back_callback = function()
        global.requestAudioStop(true, true)
        global.cleanCurrentModule()

        global.load_module(backModule.name, backModule.version).create(backModule.args)
    end
end

function global.setBackToLibrary()
    back_callback = function()
        goto_library()
    end
end

function global.mainMenu()
    goto_library()
end

function global.clean()
    global.cleanCurrentModule()
    global.screen_width = nil
    global.screen_height = nil

    global.header_height = nil

    global.visual_width = nil
    global.visual_height = nil

    global.current_module = nil
    global.current_module_name = nil
    global.progression = nil
    global.pauseData = nil
    goto_library()
end

function global.testCallBack(tcallback)
    return tcallback()
end

function global.reset_variables()
    state = {}
end

function global.get_effective_coords()
    local area = lv.area.new()
    lv.obj.get_coords(button, area)
    local button_y_min = lv.area.get_y1(area)
    return area
end

function global.load_image(path)
    local image_data = lv.img_src.load(path)
    local width = lv.img_src.get_width(image_data)
    local height = lv.img_src.get_height(image_data)
    if (enableDebug) then
        print("global.lua:112: image: " .. path)
    end
    return image_data, width, height
end

function global.table_length(table)
    local count = 0
    for _ in pairs(table) do
        count = count + 1
    end
    return count
end

function global.dumpTable(t, parent)
    for i, e in pairs(t) do
        if (type(e) == 'table') then
            global.dumpTable(e, parent .. ";" .. i)
        else
            print("global.lua:134: var: " .. parent .. ";" .. i .. "=" .. tostring(e))
        end
    end
end

function global.memLog()
    if (enableDebug ~= nil) then print("global.lua:151: plot: " .. collectgarbage("count")) end
    if (enableDebug) then global.dumpTable(state, "state") end
    --print_mem_stat()
    collectgarbage("collect")
end

function global.printMemoryUsage()
    --local mem = math.floor(collectgarbage("count")*8000) + 580270 --memory used plus system usage
    --print("======== Memory used : ".."------->" ..(100*mem)/2097152 .."%".."------->" ..mem.. " / 2097152")
    -- print(math.floor(collectgarbage("count")))
    -- print(math.floor(collectgarbage("count")*8000))
    -- print("======== Memory used : " ..mem.. " / 2097152")
    -- print("======== Memory used : " ..(100*mem)/2097152)
    --print_mem_stat()
    global.memLog()
end

function global.load_module(name, version, screenShutdownDelay, wakeUpScreen)
    --global.printMemoryUsage()
    global.removeAudioFeedbackCB() -- remove any audioFeedback function before cleaning currend module
    global.cleanCurrentModule()
    global.printMemoryUsage()

    local module

    if (version == "") then
        module = require(name)
        global.current_module_name = name
    else
        module = require(name .. "_" .. version)
        global.current_module_name = name .. "_" .. version
    end
    if (enableDebug) then print("global.lua:114: info: " .. global.current_module_name .. " Loaded") end
    if (enableDebug) then print("global.lua:114: module: " .. global.current_module_name) end
    if (screenShutdownDelay == nil) then
        screen.set_state(true)
    end
    global.current_module = module
    return module
end

function global.load_script(name, version)
    return require(name .. "_" .. version)
end

function global.strip_chars(str)
    local tableAccents = {}
    tableAccents["à"] = "a"
    tableAccents["á"] = "a"
    tableAccents["â"] = "a"
    tableAccents["ã"] = "a"
    tableAccents["ä"] = "a"
    tableAccents["ç"] = "c"
    tableAccents["è"] = "e"
    tableAccents["é"] = "e"
    tableAccents["ê"] = "e"
    tableAccents["ë"] = "e"
    tableAccents["ì"] = "i"
    tableAccents["í"] = "i"
    tableAccents["î"] = "i"
    tableAccents["ï"] = "i"
    tableAccents["ñ"] = "n"
    tableAccents["ò"] = "o"
    tableAccents["ó"] = "o"
    tableAccents["ô"] = "o"
    tableAccents["õ"] = "o"
    tableAccents["ö"] = "o"
    tableAccents["ù"] = "u"
    tableAccents["ú"] = "u"
    tableAccents["û"] = "u"
    tableAccents["ü"] = "u"
    tableAccents["ý"] = "y"
    tableAccents["ÿ"] = "y"
    tableAccents["À"] = "A"
    tableAccents["Á"] = "A"
    tableAccents["Â"] = "A"
    tableAccents["Ã"] = "A"
    tableAccents["Ä"] = "A"
    tableAccents["Ç"] = "C"
    tableAccents["È"] = "E"
    tableAccents["É"] = "E"
    tableAccents["Ê"] = "E"
    tableAccents["Ë"] = "E"
    tableAccents["Ì"] = "I"
    tableAccents["Í"] = "I"
    tableAccents["Î"] = "I"
    tableAccents["Ï"] = "I"
    tableAccents["Ñ"] = "N"
    tableAccents["Ò"] = "O"
    tableAccents["Ó"] = "O"
    tableAccents["Ô"] = "O"
    tableAccents["Õ"] = "O"
    tableAccents["Ö"] = "O"
    tableAccents["Ù"] = "U"
    tableAccents["Ú"] = "U"
    tableAccents["Û"] = "U"
    tableAccents["Ü"] = "U"
    tableAccents["Ý"] = "Y"

    local normalizedString = ''

    for strChar in string.gmatch(str, "([%z\1-\127\194-\244][\128-\191]*)") do
        if tableAccents[strChar] ~= nil then
            normalizedString = normalizedString .. tableAccents[strChar]
        else
            normalizedString = normalizedString .. strChar
        end
    end

    return normalizedString
end

function global.audioFeedback(state, second)
    global.audioState = state
    global.audioDuration = second
    if (global.audioFeedbackCallback ~= nil) then
        global.audioFeedbackCallback(state, second)
    end
    if (global.pauseContainer ~= nil and state ~= "pause") then
        global.removePauseImage()
    elseif (global.canShowPausePanel and state == "pause") then
        global.showPauseImage()
    end
end

function global.audioDelayerCallback()
    if (global.audioDelayPath ~= nil) then -- if play request
        if (audio.get_status() == "play") then
            if (global.audioNextHavePriority == true) then
                global.audioFeedbackCallback = nil
                print("Play detected stop current audio and wait next loop")
                audio.stop()
            end
        elseif (audio.get_status() == "pause") then
            global.audioFeedbackCallback = nil
            print("pause detected stop current audio and wait next loop")
            audio.stop()
        elseif (audio.get_status() == "stop") then
            print("Audio is stop, loading new audio")
            print("global.lua:261: audio: " .. global.audioDelayPath)
            if (audio.load(0, global.audioDelayPath, global.audioFeedback) == 0) then
                audio.play()
                global.audioDelayPath = nil
                global.audioFeedbackCallback = nil

                if (global.audioCB ~= nil) then
                    global.audioFeedbackCallback = global.audioCB
                    global.audioCB = nil
                end
            end
        end
    end
end

function global.requestAudioPlay(args)
    global.audioDelayPath = args.path
    global.registerAudioFeedbackCb(args.AFCb)
    global.audioNextHavePriority = args.priority
    if (args.showPause ~= nil) then
        global.canShowPausePanel = args.showPause
    else
        global.canShowPausePanel = true
    end

    if (global.audioDelayTimer ~= nil) then
        lv.timer.reset(global.audioDelayTimer)
    else
        global.audioDelayTimer = lv.timer.new(global.audioDelayerCallback, global.audioDelayValue, nil)
    end
end

function global.registerAudioFeedbackCb(AFCb)
    global.audioCB = AFCb
end

function global.removeAudioFeedbackCB()
    global.audioFeedbackCallback = nil
    global.audioCB = nil
end

function global.requestAudioStop(instant, cleancb)
    if (cleancb ~= nil and cleancb == true) then
        global.removeAudioFeedbackCB()
        global.audioDelayPath = nil
    end
    if (instant ~= nil and instant == true) then
        if (audio.get_status() ~= "stop") then
            audio.stop()
        end
    end
end

function global.sortCollectionTable(inTable)
    local resTable = {}
    local i = 1
    for _, v in pairs(inTable) do
        for _, entry in pairs(inTable) do
            if (entry.order == i) then
                table.insert(resTable, entry)
            end
        end
        i = i + 1
    end
    return resTable
end

function global.showPauseImage()
    if (global.pauseContainer == nil) then
        global.pauseContainer = lv.obj.new(window)
        lv.obj.remove_style_all(global.pauseContainer)
        lv.obj.set_size(global.pauseContainer, lv.obj.get_width(window), lv.obj.get_height(window))

        global.pauseContainerStyle = lv.style.new()
        lv.style.set_bg_color(global.pauseContainerStyle, lv.color.hex(0x000000))
        lv.style.set_bg_opa(global.pauseContainerStyle, 150)
        lv.obj.add_style(global.pauseContainer, global.pauseContainerStyle, lv.STATE_DEFAULT)

        global.pauseImage = lv.img.new(global.pauseContainer)
        lv.obj.remove_style_all(global.pauseImage)
        global.pauseData = Global.load_image("script/audio-player-pause.lif")
        lv.img.set_src(global.pauseImage, global.pauseData)
        lv.obj.align(global.pauseImage, lv.ALIGN_TOP_MID, 0, 70)
        lv.img.set_zoom(global.pauseImage, 256) -- 256 = 100%
        if (enableDebug) then
            print("global.lua:294: info:  Audio pause -> showing pause image")
        end
    end
end

function global.removePauseImage()
    if (global.pauseContainer ~= nil) then
        lv.obj.clean(global.pauseContainer)
        lv.obj.remove_style_all(global.pauseContainer)
        global.pauseContainerStyle = nil
        if (global.pauseData ~= nil) then global.pauseData = nil end
        global.pauseContainer = nil
        print("global.lua:303: info:  Audio pause -> removing pause image")
    end
end

function global.cleanCurrentModule()
    global.removePauseImage()
    if (global.current_module ~= nil) then
        print("Cleaning current module -> " .. global.current_module_name)
        global.current_module.clean()
        lv.obj.clean(window) -- ensure window is cleaned
        package.loaded[global.current_module_name] = nil
        if (enableDebug) then print("global.lua:393: info:  Modules amount :" .. global.table_length(package.loaded)) end
        global.current_module = nil
        global.current_module_name = nil
    end
    collectgarbage("collect")
end

function global.loadBranch(name)
    global.cleanCurrentModule()
    global.removeCurrentBranch()
    print("global.lua:393: info: Loading branch -> " .. name)
    global.current_branch = require(name)
    global.current_branch_name = name
    print("global.lua:393: info: Loaded -> " .. global.current_branch_name)
end

function global.removeCurrentBranch()
    Global.cleanCurrentModule()
    if (global.current_branch ~= nil) then
        print("global.lua:393: info: Cleaning current branch -> " .. global.current_branch_name)
        global.current_branch.clear()
        package.loaded[global.current_branch_name] = nil
        -- for name,v in pairs(package.loaded) do
        --     print(name)
        -- end
        -- _G[global.current_branch_name] = nil
        global.current_branch = nil
        global.current_branch_name = nil
    end
    collectgarbage("collect")
    global.memLog()
end

function global.setDefaultAudioPlayerCover(apDefaultCover, apDefaultBGCover, apDefaultFGCover)
    global.apDefaultCover = apDefaultCover
    global.apDefaultBGCover = apDefaultBGCover
    global.apDefaultFGCover = apDefaultFGCover
end

function global.loadResource(name)
    print("global.lua:393: info: Loading resource -> " .. name)
    table.insert(global.loadedResources, name)
    return require(name)
end

function global.freeResource(name)
    package.loaded[name] = nil
    for it, ressourceName in ipairs(global.loadedResources) do
        if (ressourceName == name) then
            table.remove(global.loadedResources, it)
        end
    end
    collectgarbage("collect")
    global.memLog()
end

function global.init()
    -- screen.on_state_changed(global.screenEvent)
    --lv.timer.new(global.memLog, 250, nil)
end

return global
               