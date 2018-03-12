msgpack = require 'msgpack'

ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
ws.binaryType = 'arraybuffer'


ws.onopen = (e) ->
    console.log e
    ws.send msgpack.encode({id: 0})

ws.onerror = (e) ->
    console.log e

ws.onmessage = (e) ->
    payload = msgpack.decode e.data
    console.log payload

console.log ws
