// @validatePreserveExistingMemoizationGuarantees
import {useCallback} from 'react';

function Component({services}) {
  const callback = useCallback(() => {
    return services.data?.query.timefilter.getBounds();
  }, [services.data?.query.timefilter]);

  return <Page callback={callback} />;
}
