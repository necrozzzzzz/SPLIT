"""Melee conversion checks. Run: py -m unittest discover -s tools -p test_botrec_melee.py"""
import unittest

import botrec_apply_split_route as conv


class MeleeConversionTests(unittest.TestCase):
    def timeline(self, rows):
        events = [{"tMs": t, "buttons": buttons} for t, buttons in rows]
        timeline = conv.prepare_melee_tick_events([], [], events, [t for t, _ in rows])
        return timeline, [tick for tick, _ in timeline]

    def test_native_reference_tap_and_hold(self):
        timeline, _ = self.timeline([
            (0, {}), (62 * 1000 / 64, {"melee": True}),
            (69 * 1000 / 64, {}), (204 * 1000 / 64, {"melee": True}),
            (259 * 1000 / 64, {}),
        ])
        self.assertEqual(timeline, [(62, True), (69, False), (204, True), (259, False)])

    def test_sub_tick_taps_keep_separate_press_and_release_edges(self):
        timeline, ticks = self.timeline([
            (0, {}), (5, {"melee": True}), (9, {}),
            (10, {"melee": True}), (11, {}),
        ])
        self.assertEqual(timeline, [(1, True), (2, False), (3, True), (4, False)])
        self.assertEqual([conv.melee_held_at_tick(timeline, ticks, i) for i in range(5)],
                         [False, True, False, True, False])

    def test_melee_does_not_produce_lateral_movement(self):
        event = {"buttons": {"melee": True}}
        self.assertEqual(conv.route_button_mask(event), 0x100000000)
        self.assertEqual(conv.input_movement_at_ms([], [], [event], [0], 0), (0, 0))
        event["buttons"]["left"] = True
        self.assertEqual(conv.route_button_mask(event), 0x100000200)
        self.assertEqual(conv.input_movement_at_ms([], [], [event], [0], 0), (0, 1))

    def test_high_rate_events_preserve_tap_absent_from_samples(self):
        samples = [{"buttons": {"melee": False}}, {"buttons": {"melee": False}}]
        events = [{"buttons": {"melee": True}}, {"buttons": {"melee": False}}]
        self.assertEqual(conv.prepare_melee_tick_events(samples, [0, 16], events, [5, 9]),
                         [(1, True), (2, False)])

    def test_legacy_routes_do_not_invent_melee(self):
        samples = [{"buttons": {"left": True}}, {"buttons": {"left": False}}]
        self.assertEqual(conv.prepare_melee_tick_events(samples, [0, 16], None, None), [])
        self.assertEqual(conv.route_button_mask(samples[0]), conv.BUTTON_LEFT)

    def test_native_button_delta_encodes_64_bit_melee_and_release(self):
        # One outer command carrying a tick. Check the actual nested button delta.
        command = conv.varint_field(2, 100)
        frame = conv.encode_varint(1 << 3 | 2) + conv.encode_varint(len(command)) + command
        press = conv.patch_frame_buttons(frame, "change", conv.BUTTON_MELEE, conv.BUTTON_MELEE)
        release = conv.patch_frame_buttons(frame, "change", 0, conv.BUTTON_MELEE)
        def values(frame):
            command = conv.get_command(frame)
            result = {}
            for field, wire, start, end in conv.parse_fields(command):
                if field == 3 and wire == 2:
                    nested = command[start:end]
                    for n, w, a, b in conv.parse_fields(nested):
                        result[n] = 0 if w == 7 else conv.read_varint(nested, a)[0]
            return result
        self.assertEqual(values(press), {1: 0x100000000, 2: 0x100000000})
        self.assertEqual(values(release), {1: 0, 2: 0x100000000})


if __name__ == "__main__":
    unittest.main()
