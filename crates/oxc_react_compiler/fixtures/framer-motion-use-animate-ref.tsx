import {useAnimate} from 'framer-motion';
import {useEffect} from 'react';

function Component() {
  const [scope, animate] = useAnimate();

  useEffect(() => {
    if (scope.current) {
      scope.current.style.height = 'auto';
      animate(scope.current, {opacity: 1});
    }
  }, [animate, scope]);

  return <div ref={scope} />;
}
