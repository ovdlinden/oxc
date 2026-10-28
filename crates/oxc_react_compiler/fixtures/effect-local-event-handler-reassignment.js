import {useEffect} from 'react';

function Component() {
  useEffect(() => {
    let active = true;
    const onEvent = () => {
      active = false;
    };
    window.addEventListener('event', onEvent);
    return () => {
      window.removeEventListener('event', onEvent);
      active = false;
    };
  }, []);
  return null;
}
