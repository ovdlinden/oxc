import React from 'react';

function Component({children, items}) {
  let maxLength = 0;
  const mapped = items.map(item => {
    maxLength = Math.max(maxLength, item.label.length);
    return item;
  });

  let firstMatch = null;
  const hasMatch =
    items &&
    items.some(item => {
      firstMatch = item.matches ? item : null;
      return item.matches;
    });

  let offset = 0;
  const highlighted = React.Children.map(children, child => {
    offset += child.props.children.length;
    return child;
  });

  let optionalCount = 0;
  const optionalMapped = items?.map(item => {
    optionalCount++;
    return item;
  });

  return <div data-max={maxLength} data-match={firstMatch}>{hasMatch}{mapped}{highlighted}{offset}{optionalCount}{optionalMapped}</div>;
}
