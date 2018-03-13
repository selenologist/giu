#msgpack = require 'msgpack'
console.log(d3)

encode = (data) ->
    # msgpack.encode data
    JSON.stringify(data)

decode = (data) ->
    # msgpack.decode data
    JSON.parse(data)

graph = {}
window.graph_data = graph

make_port = (node_id, port_id, port_type) ->
    console.log("making port ", node_id, port_id)
    port = {}
    port.type = port_type
    if port_type != "InPort"
        port.to = new Set()

    port.connect = (to_port) ->
        console.log("connecting", port_id, to_port)
        port.to.add(to_port)
        console.log("portset", port.to)
    port.disconnect = (to_port) ->
        port.to.delete(to_port)
    port.node = node_id
    port.id   = port_id
    graph.links[port_id] = port

find_ports = (n) ->
    for node_id, node of n.nodes
        console.log("find_ports", node, node_id)
        if node.hasOwnProperty("port")
            make_port(node_id, node.port, node.type)
        else if node.hasOwnProperty("nodes")
            find_ports(node)


opposite = (direction) ->
    if direction == "in"
        "out"
    else
        "in"

reorder = (a, b) ->
    if a.direction == "out"
        [a, b]
    else
        [b, a]

make_port_click = (send) ->
    window.port_click = (direction, id) ->
        console.log("clicked " + id)
        current_port = window.current_port
        if current_port
            console.log("currently", current_port)
            if current_port.direction != direction
                [from, to] = reorder(current_port, {direction: direction, id: id})
                send({type: "AddLink", from: from.id, to: to.id})
            window.current_port = false
        else
            current_port = {}
            current_port.direction = direction
            current_port.id        = id
            window.current_port = current_port

node_wrap = (inner) ->
    "<div class=\"Node\">"+inner+"</div>"

render_type = {
    InPort:   (name, node) ->
        "<button class=\"InPort\" id=\""+name+"\" onclick=\"port_click('in', '"+node.port+"')\"/>"
    OutPort:  (name, node) ->
        "<button class=\"OutPort\" id=\""+name+"\" onclick=\"port_click('out', '"+node.port+"')\"/>"
    Label:    (name, node) ->
        "<p class=\"Label\" id=\""+name+"\">"+graph.data[node.data].val+"</p>"
    Labelled: (name, node) =>
        str = render_type['Label'](name+'Label', node)
        for id, node of node.nodes
            str += render_type[node.type](id, node)
        node_wrap(str)
}

render_node = (name, node) ->
    render_type[node.type](name, node)

render = (graph) ->
    str = ""
    for id, node of graph.nodes
        console.log('rendering', id, node)
        str += render_node(id, node)

    document.getElementById("graph").innerHTML = str

render_links = (graph) ->
    document.getElementById("links-list").innerHTML = JSON.stringify(graph.links)


frontend = () ->
    ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
    #ws.binaryType = 'arraybuffer'

    end = 'frontend'

    buf2hex = (buf) ->
      Array.prototype.map.call(
        new Uint8Array(buf),
        (x) => ('00' + x.toString(16)).slice(-2)).join('')

    send = (obj) ->
        console.log end + ' sendobj', obj
        pack = encode(obj)
        ws.send pack

    set_graph = (r) ->
        Object.assign(graph, r.graph)
        num_nodes = Object.keys(graph.nodes).length
        if num_nodes != 0
            make_port_click(send)
            find_ports(graph)
            render(graph)

    add_link = (r) ->
        from = r.from
        to   = r.to
        graph.links[from].connect(to)
        render_links(graph)

    del_link = (r) ->
        from = r.from
        to   = r.to
        graph.links[from].disconnect(to)
        render_links(graph)

    command = {
        SetGraph: set_graph,
        AddLink:  add_link,
        DelLink:  del_link
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
        payload = decode e.data
        next = next(payload)

backend = () ->
    ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
    #ws.binaryType = 'arraybuffer'

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
                    "TestNode": {
                        type: "Labelled"
                        data: "TestLabelData"
                        nodes: {
                            "OutPortNode": {
                                type: "OutPort"
                                port: "OutPort"
                            }
                            "InPortNode": {
                                type: "InPort"
                                port: "InPort"
                            }
                        }
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
        pack = encode(obj)
        #console.log end + ' sendhex', buf2hex pack
        ws.send pack

    ws.onopen = (e) ->
        console.log end + ' open', e 

    ws.onerror = (e) ->
        console.log end + ' got error', e

    next = get_graph_list
    ws.onmessage = (e) ->
        payload = decode e.data
        #console.log(end + ' got message', payload)
        next = next(payload)

backend()

console.log ws
