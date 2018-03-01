msgpack = require 'msgpack'

ws = new WebSocket 'ws://127.0.0.1:3001'
ws.binaryType = 'arraybuffer'

ws.send(msgpack.encode({graph: 0}))

ws.onmessage = (e) ->
    payload = msgpack.decode e.data
    console.log payload
