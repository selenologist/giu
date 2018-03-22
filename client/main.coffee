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

updateLinks = false
updateNodes = false
update = false # set by d3setup, call to update after modifying state

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

mkDrag  = (n) -> (d) ->
    x = d3.event.x
    y = d3.event.y
    l = this.drag or {}
    console.log('n', n)
    switch d3.event.type
        when "start"
            rect = n.selector.node().getBoundingClientRect()
            console.log('box', rect)
            l.start = {x: rect.x, y: rect.y}
            l.end = {x: x, y: y}
            this.dragpath = d3.select('#links').append("path").classed('link', true)
        when "drag"
            l.end = {x: x, y: y}
        when "end"
            this.dragpath.remove()
            l = null
    this.dragpath.attr("d",
        "M" + l.start.x + ',' + l.start.y +
        "L" + l.end.x   + ',' + l.end.y)
    this.drag = l

addPortsToNode = (g, node_width, port_list, style) ->
    size = style.padding
    [lcount, rcount] = [0,0] # number of ports on left and right side
    yspacing = 16
    yoffset  = yspacing
    for _, port of port_list
        [cx, cy] =
            switch port.type
                when "InPort"
                    [size/2, (lcount++) * yspacing]
                when "OutPort"
                    [node_width-size/2, (rcount++) * yspacing]
                else
                    [node_width-size/2, (rcount++) * yspacing]
        port.selector =
            g.append("circle")
             .classed(port.type, true)
             .attr("cx", cx)
             .attr("cy", cy+yoffset)
             .attr("r", size)
        port.parent_g = g

        drag = mkDrag(port)
        port.selector.call(d3.drag()
                   .on("start", drag)
                   .on("drag",  drag)
                   .on("end",   drag))

    g

buildLabelled = (g, node, style) ->
    label = g.append("text")
        .classed("label", true)
        .attr("x", style.padding)
        .text(getDataAsString(node.data))
    label.attr("y", -> label.node().getBBox().height*1.1)
    node.label = label

buildNodeType = {
    Label:    buildLabelled
    Labelled: buildLabelled
}

buildNode = (g, node) ->
    style = {}
    style.padding = 5

    console.log('building', node)

    noderect = g.append("rect")
        .attr("rx", 5)
        .attr("ry", 5)

    buildNodeType[node.type](g, node, style)
    
    box = g.node().getBBox()
    width  = box.width+style.padding*2
    height = box.height+style.padding*2
    noderect.attr("width",  width)
    noderect.attr("height", height)

    portlist = Object.filter(graph.ports, (p) -> p.parent == node)
    if portlist?
        g = addPortsToNode(g, width, portlist, style)
        noderect.attr("height",
            Math.max(height, g.node().getBBox().height+style.padding*2))
    g

addNode = (parent, node_id, node) ->
    new_node = {node: node_id, type: node.type, data: node.data}
    if node.nodes?
        console.log('adding container', node_id)
        [new_node.x, new_node.y] = [500, 500]
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

d3setup = ->
    colors = d3.scaleOrdinal(d3.schemeCategory10)

    svg = d3
        .select('body')
        .append('svg')
        .attr('oncontextmenu', 'return false;')
        .attr('width',  "100%")
        .attr('height', "100%")

    nodegroup =
        svg.append("g")
           .attr("id", "nodes")
           .selectAll("g")

    linkgroup =
        svg.append("g")
           .attr("id", "links")
           .selectAll("path")
   
    nodes = false
    updateNodes = ->
        node = nodegroup.data(graph.nodes)

        node.exit()
            .remove()

        nodes = node.enter()
            .append("g")
            .attr("id", (d) -> d.node)
            .classed("node", true)
            .each((d) ->
                g = d3.select(this)
                buildNode(g, d))
   
    links = false
    updateLinks = ->
        link = linkgroup.data(graph.links)

        link.exit().remove()

        links = link.enter()
             .append("path")
             .classed("link", true)

    simulation = false
    update = ->
        updateNodes()
        updateLinks()
        if !simulation
            simulation =
                d3.forceSimulation(graph.nodes)
                  .force('charge', d3.forceManyBody()
                                     .strength(-100))
                  .force('radial', d3.forceRadial(300, 200, 200)
                                     .strength(0.5))
                  .on('tick', ->
                      # update nodes with recomputed position
                      nodes.attr("transform", (d) -> "translate(" + d.x + "," + d.y + ")")
                      nodes.datum().label?.text((d) -> getDataAsString(d.data))
                      
                      links.attr("d", (d) ->
                                  "M" + getPosition(graph.ports[d.source].selector) +
                                  "L" + getPosition(graph.ports[d.target].selector))
                  )
 
encode = (data) ->
    # msgpack.encode data
    JSON.stringify(data)

decode = (data) ->
    # msgpack.decode data
    JSON.parse(data)

frontend = ->
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
            for source, target_ports of c.graph.links
                for _, target of target_ports
                    addLink(source, target)
        update()

    set_data = (c) ->
        graph.data[c.id] = c.value

    add_link = (c) ->
        addLink(c.source, c.target)

    del_link = (c) ->
        delLink(c.source, c.target)

    command = {
        SetGraph: set_graph,
        SetData:  set_data,
        AddLink:  add_link,
        DelLink:  del_link,
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
        main_loop
    
    fatal = (r) ->
        console.log end + ' fatal ', r
        fatal

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

    d3setup()

backend = ->
    ws = new WebSocket('ws://127.0.0.1:3001', 'selenologist-node-editor')
    #ws.binaryType = 'arraybuffer'

    end = 'backend'

    fatal = (r) ->
        console.log end + ' fatal ', r
        fatal

    main_loop = (r) ->
        console.log(end + ' loop', r)
        main_loop

    set_data = (key, value) ->
        send {
            type: "SetData",
            id:    key,
            value: {val: value}
        }

    send_graph = (r) ->
        send {
            type:  "SetGraph"
            graph: {
                nodes: {
                    "TestNode": {
                        type: "Labelled"
                        data: "TestLabelData",
                        nodes: {
                            "TestNodeOut": {
                                type: "OutPort"
                            }
                            "TestNodeIn": {
                                type: "InPort"
                            }
                        }
                    },
                    "2ndNode": {
                        type: "Labelled",
                        data: "Time",
                        nodes: {
                            "2ndNodeIn": {
                                type: "InPort"
                            }
                        }
                    }
                    "AnotherNode": {
                        type: "Labelled",
                        data: "Octopus",
                        nodes: {
                            "Octopus1": {
                                type: "OutPort"
                            },
                            "Octopus2": {
                                type: "OutPort"
                            },
                            "Octopus3": {
                                type: "BiPort"
                            },
                            "Octopus4": {
                                type: "OutPort"
                            },
                            "Octopus5": {
                                type: "BiPort"
                            },
                            "Octopus6": {
                                type: "OutPort"
                            },
                            "Octopus7": {
                                type: "OutPort"
                            },
                            "Octopus8": {
                                type: "OutPort"
                            },
                        }
                    }

                }
                links: {
                    "TestNodeOut": ["2ndNodeIn", "Octopus2", "Octopus7"],
                    "Octopus1": ["TestNodeIn"]
                    "Octopus3": ["TestNodeIn"]
                    "Octopus4": ["2ndNodeIn"]
                    "Octopus5": ["2ndNodeIn"]
                }
                data:  {
                    TestLabelData: {val: "Test"},
                    Time: {val: (new Date()).toLocaleTimeString()},
                    Octopus: {val: "Octopus"}
                }
            }
        }
        window.setInterval(
            -> (set_data("Time", (new Date()).toLocaleTimeString())),
            5000)
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

reloader = ->
    ws = new WebSocket('ws://127.0.0.1:3002', 'selenologist-minimal-reloader')
    
    reload = ->
        location.reload(true)
    
    ws.onmessage = (e) ->
        if e.data == "Reload"
            reload()
        # yup that's it

    # also make ctrl-s reload the page
    d3.select("body")
      .on("keydown", ->
          if d3.event.ctrlKey && (d3.event.key == 's')
              d3.event.preventDefault()
              reload())

reloader()
backend()
