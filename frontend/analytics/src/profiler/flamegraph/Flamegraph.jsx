import React, { useEffect, useRef, useState } from 'react';
import * as d3 from 'd3';

const MAX_DEPTH = 20;

export const Flamegraph = ({
  traces,
  onBlockClick,
  styles = {},
}) => {
  const containerRef = useRef(null);
  const [highlightedBlock, setHighlightedBlock] = useState(null);

  useEffect(() => {
    if (!containerRef.current || traces.length === 0) return;

    const svgRef = containerRef.current;
    const width = svgRef.clientWidth || 800;
    const height = svgRef.clientHeight || 600;

    const processedTraces = traces.map((trace) => ({
      id: trace.contractId || trace.id || 'unknown',
      name: trace.contractName || trace.name || 'Unknown Contract',
      cost: trace.instructionCount || trace.gasUsed || 0,
      children: (trace.subcalls || trace.children || []).map((c) => ({
        id: c.contractId || c.id || 'unknown',
        name: c.contractName || c.name || 'Unknown Sub-call',
        cost: c.instructionCount || c.gasUsed || 0,
        children: [],
      })),
    }));

    const root = d3.hierarchy({
      id: 'root',
      name: 'Root',
      cost: 0,
      children: [processedTraces[0] || { id: 'root', name: 'No Data', cost: 0, children: [] }],
    });

    const treemap = d3.treemap()
      .size([width, height])
      .padding(4)
      .paddingTop(20);

    treemap(root);

    const tooltip = d3.select('body').append('div')
      .style('position', 'fixed')
      .style('background', 'rgba(0, 0, 0, 0.8)')
      .style('color', '#fff')
      .style('padding', '12px 16px')
      .style('border-radius', '6px')
      .style('pointer-events', 'none')
      .style('z-index', '1000')
      .style('display', 'none');

    const elements = d3.select(svgRef)
      .selectAll('rect')
      .data(root.descendants())
      .enter()
      .append('rect')
      .attr('x', (d) => d.x0)
      .attr('y', (d) => d.y0)
      .attr('width', (d) => d.x1 - d.x0)
      .attr('height', (d) => d.y1 - d.y0)
      .attr('fill', (d) => {
        const fillColor = d3.interpolateViridis(d.data.cost / 10000);
        return fillColor;
      })
      .style('stroke', '#eeeeee')
      .style('stroke-width', '0.5px')
      .on('mouseover', (event, d) => {
        tooltip.style('display', 'block');
        tooltip.html(`
          <strong>Contract:</strong> ${d.data.name}<br />
          <strong>Instruction Cost:</strong> ${d.data.cost}<br />
          <strong>Depth:</strong> ${d.depth}
        `);
        tooltip.style('left', (event.pageX + 10) + 'px');
        tooltip.style('top', (event.pageY - 28) + 'px');
      })
      .on('mousemove', (event) => {
        tooltip.style('left', (event.pageX + 10) + 'px');
        tooltip.style('top', (event.pageY - 28) + 'px');
      })
      .on('mouseout', () => {
        tooltip.style('display', 'none');
      })
      .on('click', (event, d) => {
        setHighlightedBlock(d.data);
        if (onBlockClick) {
          onBlockClick(d.data);
        }
      });

    const labels = d3.select(svgRef)
      .selectAll('text')
      .data(root.descendants())
      .enter()
      .append('text')
      .attr('x', (d) => (d.x0 + d.x1) / 2)
      .attr('y', (d) => (d.y0 + d.y1) / 2)
      .attr('dy', '0.35em')
      .attr('text-anchor', 'middle')
      .style('font-size', (d) => {
        const width = d.x1 - d.x0;
        return width > 100 ? '11px' : width > 50 ? '9px' : '6px';
      })
      .style('fill', '#333')
      .text((d) => d.data.name.slice(0, 12));

    return () => {
      d3.select(svgRef).selectAll('*').remove();
      d3.select('body').select(tooltip.node().parentNode).remove();
    };
  }, [traces, onBlockClick]);

  const handleBlockClick = (d) => {
    if (onBlockClick) {
      onBlockClick(d);
    }
    setHighlightedBlock(d);
  };

  return (
    <div
      ref={containerRef}
      style={{
        width: styles.width || '100%',
        height: styles.height || '600px',
        border: styles.border || '1px solid #dee2e6',
        borderRadius: styles.borderRadius || '8px',
        backgroundColor: styles.backgroundColor || '#fff',
        overflow: 'hidden',
        position: 'relative',
      }}
    >
      {highlightedBlock && (
        <div
          style={{
            position: 'absolute',
            top: 20,
            right: 20,
            background: 'rgba(0, 0, 0, 0.8)',
            color: '#fff',
            padding: '12px 16px',
            borderRadius: '6px',
            fontFamily: 'sans-serif',
            zIndex: 1000,
            maxWidth: '300px',
          }}
        >
          <strong>Block Details:</strong><br />
          <div>Contract: {highlightedBlock.name}</div>
          <div>Instruction Cost: {highlightedBlock.cost}</div>
          <div>Sub-calls: {highlightedBlock.children.length}</div>
        </div>
      )}
    </div>
  );
};

export default Flamegraph;