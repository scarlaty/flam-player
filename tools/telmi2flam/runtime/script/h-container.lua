local h_container = {}

function h_container.create(parent, spacing, clickable)

    local container

    if (clickable) then
        container = lv.btn.new(parent)
    else
        container = lv.obj.new(parent)
    end

    lv.obj.remove_style_all(container)

    lv.obj.set_flex_flow(container, lv.FLEX_FLOW_ROW)
    lv.obj.set_style_pad_column(container, spacing, lv.STATE_DEFAULT)

    return container

end

return h_container