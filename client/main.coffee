msgpack = require 'msgpack'

graph = {}

opposite = (direction) ->
    if direction == "in"
        "out"
    else
        "in"

make_port_click = (ws) ->
    window.port_click = (direction, id) ->
        console.log("clicked " + id)
        current_port = window.current_port
        if window.current_port
            console.log("currently", current_port)
            if current_port.direction == opposite(direction)
                return console.log("this is where we would connect" + current_port.id + " and " + id)
            window.current_port = false
        else
            current_port = new Object
            current_port.direction = direction
            current_port.id        = id
            window.current_port = current_port

render_type = {
    InPort: (data, name, id) ->
        "<button class=\"InPort\" id=\""+name+"\" onclick=\"port_click('in', '"+id+"')\"></div>"
    OutPort: (data, name, id) ->
        "<button class=\"OutPort\" id=\""+name+"\" onclick=\"port_click('out', '"+id+"')\"></div>"
    Label: (data, name, id) ->
        "<p class=\"Label\" id=\""+name+"\">"+data[id].val+"</p>"
}

node_wrap = (inner) ->
    "<div class=\"Node\">"+inner+"</div>"

render_node = (data, name, node) ->
    node_wrap(render_type[node.type](data, name, node.id))

render = (graph) ->
    console.log('render ', graph)
    document.getElementById("graph").innerHTML =
        Object.keys(graph.nodes).map((k, _) ->
            render_node(graph.data, k, graph.nodes[k]))
                .join('<br/>')

frontend = () ->
    ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
    ws.binaryType = 'arraybuffer'

    end = 'frontend'

    buf2hex = (buf) ->
      Array.prototype.map.call(
        new Uint8Array(buf),
        (x) => ('00' + x.toString(16)).slice(-2)).join('')

    send = (obj) ->
        console.log end + ' sendobj', obj
        pack = msgpack.encode(obj)
        console.log end + ' sendhex', buf2hex pack
        ws.send pack

    set_graph = (r) ->
        render(r.graph)
        ws.close()

    command = {
        SetGraph: set_graph
    }

    process_command = (r) ->
        command[r.val.type](r.val)

    process_warn = (r) ->
        console.log(end + ' [warn]', r)

    process_err = (r) ->
        console.log(end + ' [err]', r)

    process = {
        Command: process_command
        Warn:    process_warn
        Err:     process_err
    }

    process_unknown = (r) ->
        console.log(end + ' [unknown]', r)

    main_loop = (r) ->
        p = process[r.type]
        if !p
            process_unknown(r)
        else
            p(r)
        main_loop
    
    fatal = (r) ->
        console.log end + ' fatal ', r
        this

    get_graph_list = (r) ->
        if r.list
            console.log(end + ' got graph list', r.graphs)
            send {
                type: "FrontendAttach"
                id:   0
            }
            main_loop
        else
            fatal(r)

    ws.onopen = (e) ->
        console.log end + ' opened', e 

    ws.onerror = (e) ->
        console.log end + ' error', e

    next = get_graph_list
    ws.onmessage = (e) ->
        payload = msgpack.decode e.data
        next = next(payload)

backend = () ->
    ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
    ws.binaryType = 'arraybuffer'

    end = 'backend'

    fatal = (r) ->
        console.log end + ' fatal ', r
        this

    main_loop= (r) ->
        console.log(end + ' loop', r)
        main_loop

    send_graph = (r) ->
        send {
            type:  "SetGraph"
            graph: {
                nodes: {
                    "0TestLabelNode": {
                        type: "Label"
                        id: "TestLabelData"
                    }
                    "OutPortNode": {
                        type: "OutPort"
                        id: "OutPort"
                    }
                    "InPortNode": {
                        type: "InPort"
                        id: "InPort"
                    }
                }
                links: {}
                data:  {TestLabelData: {val: "Test"}}
            }
        }
        main_loop
       

    get_graph_list = (r) ->
        if r.list
            console.log(end + ' got graph list', r.graphs)
            send {
                type: "BackendAttach"
                id:   0
            }
            frontend()
            send_graph()
        else
            fatal(r)

    buf2hex = (buf) ->
      Array.prototype.map.call(
        new Uint8Array(buf),
        (x) => ('00' + x.toString(16)).slice(-2)).join('')

    send = (obj) ->
        console.log end + ' sendobj', obj
        pack = msgpack.encode(obj)
        console.log end + ' sendhex', buf2hex pack
        ws.send pack

    ws.onopen = (e) ->
        console.log end + ' open', e 

    ws.onerror = (e) ->
        console.log end + ' got error', e

    next = get_graph_list
    ws.onmessage = (e) ->
        payload = msgpack.decode e.data
        #console.log(end + ' got message', payload)
        next = next(payload)

backend()

console.log ws
