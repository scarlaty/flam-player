require("test_helpers")
print("--- test_audio_bindings ---")

test("audio global exists", function()
    expect_type(audio, "table", "audio type")
end)

test("audio.load exists", function()
    expect_type(audio.load, "function")
end)

test("audio.play exists", function()
    expect_type(audio.play, "function")
end)

test("audio.stop exists", function()
    expect_type(audio.stop, "function")
end)

test("audio.pause exists", function()
    expect_type(audio.pause, "function")
end)

test("audio.seek exists", function()
    expect_type(audio.seek, "function")
end)

test("audio.duration returns number", function()
    local d = audio.duration()
    expect_type(d, "number", "duration type")
end)

test("audio.get_status returns string", function()
    local s = audio.get_status()
    expect_type(s, "string", "status type")
    expect_eq(s, "stop", "initial status")
end)

test("audio.load with args does not crash", function()
    audio.load(1, "test.mp3")
    expect_true(true)
end)

test("audio.play/stop/pause do not crash", function()
    audio.play()
    audio.pause()
    audio.stop()
    expect_true(true)
end)

test("audio.seek with number does not crash", function()
    audio.seek(5.0)
    expect_true(true)
end)
