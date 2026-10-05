(function () {
    'use strict';

    var shared = $.SplitPanorama = $.SplitPanorama || {};
    var modules = shared.modules = shared.modules || {};

    modules.createState = function () {
        var current = {
            protocolVersion: 1,
            sequence: -1,
            visibility: 'hidden',
            activePreset: 0,
            activePresetName: '',
            metadataRevision: 0,
            slots: [],
            canUndo: false,
            canRedo: false,
            connected: false
        };
        var listeners = [];

        function publish() {
            for (var index = 0; index < listeners.length; index += 1) {
                try { listeners[index](current); }
                catch (error) { $.Warning('[SPLIT] state listener failed: ' + error); }
            }
        }

        return {
            get: function () { return current; },
            subscribe: function (listener) {
                listeners.push(listener);
                listener(current);
            },
            apply: function (next) {
                if (!next || next.protocolVersion !== 1) return false;
                if (typeof next.sequence !== 'number' || next.sequence <= current.sequence) return false;
                if (next.visibility !== 'hidden' && next.visibility !== 'passive' &&
                    next.visibility !== 'interactive') return false;
                next.connected = true;
                current = next;
                publish();
                return true;
            },
            mergeMetadataItem: function (metadata) {
                if (!metadata || metadata.protocolVersion !== 1 ||
                    metadata.revision !== current.metadataRevision ||
                    metadata.activePreset !== current.activePreset ||
                    typeof metadata.item !== 'number' || metadata.item < 0 ||
                    metadata.item > 8 || typeof metadata.value !== 'string') return false;
                if (metadata.item === 0) {
                    current.activePresetName = metadata.value;
                } else {
                    for (var index = 0; index < current.slots.length; index += 1) {
                        if (current.slots[index].index === metadata.item) {
                            current.slots[index].name = metadata.value;
                            break;
                        }
                    }
                }
                publish();
                return true;
            },
            disconnect: function () {
                current = {
                    protocolVersion: 1,
                    sequence: -1,
                    visibility: 'hidden',
                    activePreset: 0,
                    activePresetName: '',
                    metadataRevision: 0,
                    slots: [],
                    canUndo: false,
                    canRedo: false,
                    connected: false
                };
                publish();
            }
        };
    };
})();
