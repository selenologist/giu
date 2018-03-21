Object.filter = (obj, pred) ->
    result = []
    for k, v of obj
        p = pred(v, k)
        if p then result.push(v)
    result

graph = {}
graph.nodes = []
graph.links = []
graph.ports = {}
graph.data  = {}
graph.id_to_idx = {}
window.graph_data = graph

rebuildLinks = false
rebuildNodes = false
rebuild = false # set by d3setup, call to update after modifying state

getPosition = (sel) ->
    r = sel.node().getBoundingClientRect()
    x = r.x - r.width/2
    y = r.y - r.height/2
    x.toString() + ',' + y.toString()

getDataAsString = (id) ->
    data = graph.data[id]
    if data?
        data = data.val
        switch typeof data
            when "string"
                data
            when "number", "boolean"
                data.toString()
            when "undefined"
                "[undefined]"
            when "object"
                if data? # if not null
                    JSON.stringify data
                else
                    "[null]"
            when "function"
                "[wtf why is a function in graph.data]"

drag  = (n) ->
    n = d.append("path")
    console.log('d', n)
    x = d3.event.x
    y = d3.event.y
    switch d3.event.type
        when "start"
            n.start = {x,y}
            n.end   = {x,y}
        when "drag"
            n.end = {x,y}
        when "stop"
            n.remove()
    n.attr("d",
        "M" + n.start.x + ',' + n.start.y +
        "L" + n.end.x   + ',' + n.end.y)

addPortsToNode = (g, node_width, port_list, style) ->
    size = style.padding
    [lcount, rcount] = [0,0] # number of ports on left and right side
    for _, port of port_list
        yspacing = 20
        [color, cx, cy] =
            switch port.type
                when "InPort"
                    ["blue", size/2, (lcount++) * yspacing]
                when "OutPort"
                    ["red", node_width-size/2, (rcount++) * yspacing]
                else
                    ["green", node_width-size/2, (rcount++) * yspacing]
        port.selector =
            g.append("circle")
             .attr("cx", cx)
             .attr("cy", cy)
             .attr("r", size)
             .attr("fill", color)
             .call((d) ->
                 n = d
                 d3.drag()
                    .on("start", -> drag(n))
                    .on("drag",  -> drag(n))
                    .on("end",   -> drag(n)))
        port.parent_g = g
    g

buildLabelled = (g, node, style) ->
    label = g.append("text")
        .attr("fill", "red")
        .attr("x", style.padding)
        .text(getDataAsString(node.data))
    label.attr("y", () -> label.node().getBBox().height)

buildNodeType = {
    Label:    buildLabelled
    Labelled: buildLabelled
}

buildNode = (g, node) ->
    style = {}
    style.padding = 4

    console.log('building', node)

    noderect = g.append("rect")
        .attr("rx", 5)
        .attr("ry", 5)

    buildNodeType[node.type](g, node, style)
    
    box = g.node().getBBox()
    width  = box.width+style.padding*2
    height = box.height+style.padding*2
    noderect.attr("width",  width)
            .attr("height", height)

    portlist = Object.filter(graph.ports, (p) -> p.parent == node)
    if portlist?
        g = addPortsToNode(g, width, portlist, style)
    g.attr("transform", (d) ->
        [d.x, d.y] = [200, 200]
        "translate(" + d.x + "," + d.y + ")")
    g

addNode = (parent, node_id, node) ->
    if node.nodes?
        console.log('adding container', node_id)
        new_node = {node: node_id, type: node.type, data: node.data}
        parent = parent ? new_node
        enumerateNodes(parent, node.nodes)
        graph.nodes.push new_node
    else switch node.type
        when 'InPort', 'OutPort', 'BiPort'
            port = node_id
            console.log('adding port', port)
            graph.ports[port] = {
                 type: node.type
                 port: port
                 parent: parent
                 selector: null
                 parent_g: null
            }
        else
            console.log('adding regular node', node_id)
            new_node = {node: node_id, type: node.type, data: node.data}
            if parent
                parent.nodes.push new_node
            else
                graph.nodes.push new_node
                graph.id_to_idx[node_id] = graph.nodes.length - 1
    return

enumerateNodes = (parent, nodes) ->
    for node_id, node of nodes
        addNode(parent, node_id, node)
    return

addLink = (source, target) ->
    console.log("adding link between ", source, 'and', target)
    graph.links.push({
        source: source,
        target: target
    })

delLink = (source, target) ->
    graph.links.splice(
        graph.links.find( (l) ->
            l.source == source && target == target),
            1)


d3setup = () ->
    colors = d3.scaleOrdinal(d3.schemeCategory10)

    svg = d3
        .select('body')
        .append('svg')
        .attr('oncontextmenu', 'return false;')
        .attr('width',  "100%")
        .attr('height', "100%")

    nodegroup = svg.append("g")
        .attr("class", "nodes")
        .selectAll(".nodes")
   
    tick = () ->
        # update nodes with recomputed position
        for _, d of graph.nodes
          d3.select("g#" + d.node)
            .attr("transform", () ->
                "translate(" + d.x + "," + d.y + ")")
        return

    rebuildNodes = () ->
        node = nodegroup
            .data(graph.nodes)
            .enter().append("g")
            .attr("id", (d) -> d.node)
            .each((d) ->
                g = d3.select(this)
                buildNode(g, d))
        simulation = d3.forceSimulation(graph.nodes)
            .force('charge', d3.forceManyBody())
            .force('center', d3.forceCenter(300, 300))
            .on('tick', tick)
        

    rebuildLinks = () ->
        linkgroup = svg.append("g")
            .attr("class", "links")
            .selectAll("path")

        links = linkgroup
                .data(graph.links, (d) ->
                    x = d.source + "#" + d.target
                    console.log('x', x)
                    x)

        links.exit().remove()

        links.enter()
             .append("path")
             .attr("stroke", "yellow")
             .attr("d", (d) ->
                "M" + getPosition(graph.ports[d.source].selector) +
                "L" + getPosition(graph.ports[d.target].selector))

        window.setTimeout(rebuildLinks, 500)

    rebuild = () ->
        rebuildNodes()
        rebuildLinks()
    
encode = (data) ->
    # msgpack.encode data
    JSON.stringify(data)

decode = (data) ->
    # msgpack.decode data
    JSON.parse(data)

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
    window.send = send

    set_graph = (c) ->
        num_nodes = Object.keys(c.graph).length
        graph.data = c.graph.data
        if num_nodes != 0
            enumerateNodes(null, c.graph.nodes)
            if !rebuildNodes
                d3setup()
            for source, target_ports of c.graph.links
                for _, target of target_ports
                    addLink(source, target)

    add_link = (c) ->
        addLink(c.source, c.target)

    del_link = (c) ->
        delLink(c.source, c.target)

    reload = (c) ->
        location.reload()

    command = {
        SetGraph: set_graph,
        AddLink:  add_link,
        DelLink:  del_link,
        Reload:   reload
    }

    process_command = (r) ->
        c = command[r.val.type]
        if c?
            c(r.val)
        else
            console.log('unknown command', r.val.type)

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
        console.log('mainloop', r)
        p = process[r.type]
        if p?
            p(r)
        else
            process_unknown(r)
        if rebuild
            rebuild()
        main_loop
    
    fatal = (r) ->
        console.log end + ' fatal ', r
        this

    get_graph_list = (r) ->
        if r.list
            console.log(end + ' got graph list', r.list)
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
                            "TestNodeOut": {
                                type: "OutPort"
                            }
                            "TestNodeIn": {
                                type: "InPort"
                            }
                        }
                    }
                }
                links: {
                    "TestNodeOut": ["TestNodeIn"]
                }
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
