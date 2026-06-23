local v_scroll = {}

v_scroll.focusable_objects = {}
v_scroll.current_focused_obj = nil
v_scroll.current_focused_index = nil
v_scroll.scroll_direction = 0
v_scroll.scroll_step = 80
v_scroll.target = nil

v_scroll.parent_container = nil
v_scroll.container_to_scroll = nil

v_scroll.parent_container_event_key_cb = nil
v_scroll.container_to_scroll_scroll_end_cb = nil
v_scroll.parent_container_clicked_cb = nil

function v_scroll.clean()

    v_scroll.focusable_objects = {}
    v_scroll.target = nil
    v_scroll.scroll_direction = 0
    v_scroll.current_focused_obj = nil
    v_scroll.current_focused_index = nil

    if (v_scroll.parent_container ~= nil) then
        lv.obj.remove_event_cb(v_scroll.parent_container, v_scroll.parent_container_event_key_cb)
        lv.obj.remove_event_cb(v_scroll.parent_container, v_scroll.parent_container_clicked_cb)
        v_scroll.parent_container = nil
    end

    if (v_scroll.container_to_scroll ~= nil) then
        lv.obj.remove_event_cb(v_scroll.container_to_scroll, v_scroll.container_to_scroll_scroll_end_cb)
        v_scroll.container_to_scroll = nil
    end

end

function v_scroll.defocus_current_object()

    if (v_scroll.current_focused_obj == nil) then
        do
            return
        end
    end

    lv.obj.clear_state(v_scroll.current_focused_obj, lv.STATE_FOCUSED)
    lv.event.send(v_scroll.current_focused_obj, lv.EVENT_DEFOCUSED, nil)

    v_scroll.current_focused_obj = nil

end

function v_scroll.focus_obj(obj)

    v_scroll.defocus_current_object()

    for i, v in ipairs(v_scroll.focusable_objects) do
        if (v == obj) then
            v_scroll.current_focused_index = i
        end
    end

    v_scroll.current_focused_obj = obj
    lv.obj.add_state(obj, lv.STATE_FOCUSED)
    lv.event.send(obj, lv.EVENT_FOCUSED, nil)

end

function v_scroll.clicked_cb()

    if (v_scroll.current_focused_obj ~= nil) then
        lv.event.send(v_scroll.current_focused_obj, lv.EVENT_CLICKED, nil)
    end

end

function v_scroll.pressed_cb()

    if (v_scroll.current_focused_obj ~= nil) then
        lv.event.send(v_scroll.current_focused_obj, lv.EVENT_PRESSED, nil)
    end

end

function v_scroll.scroll_end_cb()
end

function v_scroll.compute_cont_self_height(container)

    local area = lv.area.new()
    local self_height = 0
    local child_count = lv.obj.get_child_cnt(container)
    local max_y_child = 0

    for i = 0, child_count - 1 do

        local child = lv.obj.get_child(container, i)
        lv.obj.get_coords(child, area)
        local y_max = lv.area.get_y2(area)
        self_height = math.max(max_y_child, y_max)

    end

    return self_height
end

function v_scroll.find_nearest_obj(vpos, elements)

    local area = lv.area.new()
    local min_distance = 999999
    local nearest_child_index = 0
    local nearest_child = nil

    for i, child in ipairs(elements) do

        lv.obj.get_coords(child, area)
        local y_min = lv.area.get_y1(area)
        local distance = math.abs(vpos - y_min)

        if (distance < min_distance) then

            min_distance = distance
            nearest_child_index = i
            nearest_child = child

        end

    end

    return nearest_child_index, nearest_child

end

function v_scroll.event_key_cb(event)

    local code = lv.event.get_code(event)

    if (code == lv.EVENT_KEY) then

        local direction = string.byte(lv.event.get_key_value(event))
        local step = 1

        if (direction == lv.KEY_LEFT) then
            step = 1
        elseif (direction == lv.KEY_RIGHT) then
            step = -1
        else
            do
                return
            end
        end

        v_scroll.scroll_direction = step
        local table_size = Global.table_length(v_scroll.focusable_objects)

        if (v_scroll.current_focused_index ~= nil) then
            if (v_scroll.current_focused_index > table_size) then
                v_scroll.current_focused_index = nil
            elseif (v_scroll.current_focused_index < 1) then
                v_scroll.current_focused_index = nil
            end
        end

        local area = lv.area.new()
        local cont_self_height = v_scroll.compute_cont_self_height(v_scroll.container_to_scroll) + 16
        local scroll_y = lv.obj.get_scroll_y(v_scroll.container_to_scroll)
        local scroll_increment = step * v_scroll.scroll_step
        local target_scroll = scroll_y - scroll_increment

        if (target_scroll < 0) then
            scroll_increment = scroll_increment + target_scroll
        elseif ((cont_self_height + scroll_increment) < Global.screen_height) then
            scroll_increment = Global.screen_height - cont_self_height
        end

        target_scroll = scroll_y - scroll_increment
        local next_focused_index

        if (v_scroll.current_focused_index == nil) then

            local nearest_obj_index, nearest_obj = v_scroll.find_nearest_obj(0, v_scroll.focusable_objects)
            next_focused_index = nearest_obj_index

        else

            if (v_scroll.scroll_direction == 1) then
                next_focused_index = v_scroll.current_focused_index - 1
            else
                next_focused_index = v_scroll.current_focused_index + 1
            end

        end

        if (next_focused_index > Global.table_length(v_scroll.focusable_objects) or next_focused_index < 1) then

            v_scroll.target = nil

        else

            local next_focused_obj = v_scroll.focusable_objects[next_focused_index]
            lv.obj.get_coords(next_focused_obj, area)
            local next_focused_obj_pos_y = lv.area.get_y1(area)

            --object is visible in screen
            if (next_focused_obj_pos_y < Global.screen_height) then

                --scroll if the object position if after the half of the screen
                if (next_focused_obj_pos_y > (Global.screen_height / 2.0)) then
                    local scroll_amount = next_focused_obj_pos_y - math.floor(Global.screen_height / 2)
                    lv.obj.scroll_by(v_scroll.container_to_scroll, 0, -scroll_amount, lv.ANIM_ON)
                end

                v_scroll.target = next_focused_obj
                v_scroll.current_focused_index = next_focused_index
                v_scroll.focus_obj(v_scroll.target)

                do
                    return
                end

            end

        end

        if (target_scroll >= 0) then
            lv.obj.scroll_by(v_scroll.container_to_scroll, 0, scroll_increment, lv.ANIM_ON)
        end


    end
end

function v_scroll.add_focusable_obj(obj)
    table.insert(v_scroll.focusable_objects, obj)
end

function v_scroll.init(_parent_container, _container_to_scroll)
    v_scroll.parent_container = _parent_container
    v_scroll.container_to_scroll = _container_to_scroll

    v_scroll.parent_container_event_key_cb = lv.obj.add_event_cb(v_scroll.parent_container, v_scroll.event_key_cb, lv.EVENT_KEY)
    v_scroll.container_to_scroll_scroll_end_cb = lv.obj.add_event_cb(v_scroll.container_to_scroll, v_scroll.scroll_end_cb, lv.EVENT_SCROLL_END)
    v_scroll.parent_container_clicked_cb = lv.obj.add_event_cb(v_scroll.parent_container, v_scroll.clicked_cb, lv.EVENT_CLICKED)
    lv.group.add_obj(document, v_scroll.parent_container)
    lv.group.focus_obj(v_scroll.parent_container)

    lv.group.set_editing(document, true)
    lv.obj.scroll_to(v_scroll.container_to_scroll, 0, 0, lv.ANIM_ON)
end

return v_scroll