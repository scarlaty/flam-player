local v_container = {}

function v_container.create(parent, spacing, clickable)

    local container

    if (clickable) then
        container = lv.btn.new(parent)
    else
        container = lv.obj.new(parent)
    end

    lv.obj.remove_style_all(container)

    lv.obj.set_flex_flow(container, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_style_pad_row(container, spacing, lv.STATE_DEFAULT)

    return container

end

return v_container