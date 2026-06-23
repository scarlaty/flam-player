ButtonTheme = {}

function ButtonTheme:new(o)
    o = o or {}
    setmetatable(o, self)
    self.__index = self
    return o
end

function ButtonTheme:get_default_style(size)
    return lv.style.new()
end

function ButtonTheme:get_focused_style(size)
    return lv.style.new()
end

function ButtonTheme:get_pressed_style(size)
    return lv.style.new()
end

return ButtonTheme