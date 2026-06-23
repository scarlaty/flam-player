
---@module 'global'
--- This module holds all progression relative manipulations
--- It manage :
--- - Progression save using setProgression()
--- - Calls to display chapters using displayChapters()
--- - Progression state usinge isStoryStarted()
--- - Progression progress percentil using getProgressionValue()
---@class progressionManager
local progressionManager =  {}

--- save back button callback
progressionManager.backButtonBehavior = nil
--- save save total amount of chapter
progressionManager.totalChapters = nil
progressionManager.chapterViewLabel = nil

--- Set back button behavior according to backButtonBehavior callback
---@return nil
function progressionManager.restoreBackBehavior()
    Global.setBackBehavior(progressionManager.backButtonBehavior)
end

---@param progressionData { currentFunction: function|string, branch?:string,ischapter?: boolean ,chapterData? :{label: string,cb: function|string,img:string,inventory:table, audio:string, order?: number}}
---@return nil
function progressionManager.setProgression(progressionData)
    progressionManager.restoreBackBehavior()
    --print("progressionManager : Current function : " .. progressionData.currentFunction)
    print("progressionManager.lua:28: node:  Current function : " .. progressionData.currentFunction)
    state.current_fun = progressionData.currentFunction
    if(progressionData.branch ~= nil)then
        state.currentBranchName = progressionData.branch
    else
        state.currentBranchName = Global.current_branch_name
    end
    
    if(state.visited_funs[progressionData.currentFunction] == nil) then
        state.visited_funs[progressionData.currentFunction] = {}
    end
    
    if (progressionData.ischapter ~= nil and progressionData.ischapter == true) then

        local loadedTable = (progression.load("chaps"))
        for pos, v in pairs(loadedTable) do
            if (v.label == progressionData.chapterData.label) then
                do
                    table.remove(loadedTable,pos)
                    --return
                end
            end
        end
        
        local chapDataCopy = {}
        chapDataCopy.label = progressionData.chapterData.label
        chapDataCopy.branch =  state.currentBranchName
        chapDataCopy.cb = progressionData.chapterData.cb
        chapDataCopy.img = progressionData.chapterData.img
        chapDataCopy.audio = progressionData.chapterData.audio
        if(progressionData.chapterData.order  ~= nil) then
            chapDataCopy.order = progressionData.chapterData.order
        end
        if(progressionData.chapterData.inventory ~= nil ) then
            chapDataCopy.inventory = {}
            for k, v in pairs(progressionData.chapterData.inventory) do
            local entry = {}
            entry.saved = {}
            for l, j in pairs(v.saved) do
                    entry.saved[l] = j
            end
            
            chapDataCopy.inventory[k] = entry
            end
        end

        table.insert(loadedTable,chapDataCopy)
        progression.save("chaps",loadedTable)
    end
    
    progress = progressionManager.getProgressionValue()
end

--- - call chapterView module for chapter Selection 
---@see chapterView
---@return nil
function progressionManager.displayChapters(forceAudio)
    Global.load_module("chapterView", "").create(progressionManager.backButtonBehavior,progressionManager.chapterViewLabel,forceAudio)
end

---@return boolean StoryStarted true if state.visited_funs is not empty 
function progressionManager.isStoryStarted()
    if(state.current_fun ~=nil and state.visited_funs ~= nil) then
        if (Global.table_length(state.visited_funs) == 0) then
            return false
        else
            return true
        end
    else
        return false
    end
end

---@return number progression current progress using visited_funs and totalChapters
function progressionManager.getProgressionValue()

    if(state.current_fun ~= nil ) then
        return math.floor(Global.table_length(progression.load("chaps")) / progressionManager.totalChapters * 100)
    else
        return 0
    end

end

--- - progressionData.totalChapters -> Total chapters of the story
--- - progressionData.backBehavior -> function to call with back button pressed
---@param progressionData { totalChapters: integer, backBehavior: function ,chapterViewLabel? : string}
---@return nil
function progressionManager.create(progressionData) 
    progressionManager.totalChapters = progressionData.totalChapters
    progressionManager.backButtonBehavior = progressionData.backBehavior
    progressionManager.chapterViewLabel = progressionData.chapterViewLabel 

    if(Global.table_length(progression.load("chaps")) == 0) then
        local visited_chapters = {}
        progression.save("chaps",visited_chapters)
    end

    if (state.visited_funs == nil) then
        state.visited_funs = {}
    end
    if (state.registeredChoices == nil) then
        state.registeredChoices = {}
    end
    progressionManager.restoreBackBehavior()
end

function progressionManager.resetProgress()
    local visited_chapters = {}
    progression.save("chaps",visited_chapters)
end


return progressionManager